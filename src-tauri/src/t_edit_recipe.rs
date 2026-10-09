//! Persist edits independently of original bytes; shared by display and export.
use serde_json::{json, Value};
use crate::{t_image, t_sqlite};
use std::sync::OnceLock;

static RENDERING: OnceLock<tokio::sync::Semaphore> = OnceLock::new();

pub fn state(edits: Option<&str>) -> Value {
    edits.and_then(|s| serde_json::from_str(s).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

pub fn recipe(edits: Option<&str>) -> Option<t_image::EditParams> {
    serde_json::from_value(state(edits).get("recipe")?.clone()).ok()
}

/// Map original-pixel corners in precisely the order used by the image renderer.
/// Returns the visible bounding rectangle and edited display dimensions.
pub fn map_bbox(b: [f64;4], width: u32, height: u32, orientation: i32, display: bool, edits: Option<&str>) -> Option<([f64;4], [u32;2])> {
    let mut points=[(b[0],b[1]),(b[0]+b[2],b[1]),(b[0],b[1]+b[3]),(b[0]+b[2],b[1]+b[3])];
    let (raw_w,raw_h)=(width as f64,height as f64);
    if !display {
        for (x,y) in &mut points {
            (*x,*y)=match orientation {
                2=>(raw_w-*x,*y),3=>(raw_w-*x,raw_h-*y),4=>(*x,raw_h-*y),
                5=>(*y,*x),6=>(raw_h-*y,*x),7=>(raw_h-*y,raw_w-*x),8=>(*y,raw_w-*x),
                _=>(*x,*y),
            };
        }
    }
    let (mut w,mut h)=if (5..=8).contains(&orientation) {(raw_h,raw_w)} else {(raw_w,raw_h)};
    let doc=state(edits);
    if let Some(p)=doc.get("recipe").filter(|p|p.is_object()) {
        for (x,y) in &mut points {
            if p["flipHorizontal"].as_bool().unwrap_or(false) {*x=w-*x;}
            if p["flipVertical"].as_bool().unwrap_or(false) {*y=h-*y;}
        }
        let angle=p["rotate"].as_i64().unwrap_or(0).rem_euclid(360);
        for (x,y) in &mut points {
            (*x,*y)=match angle {90=>(h-*y,*x),180=>(w-*x,h-*y),270=>(*y,w-*x),_=>(*x,*y)};
        }
        if angle==90 || angle==270 {std::mem::swap(&mut w,&mut h);}
        let a=p["cropAngle"].as_f64().unwrap_or(0.0) as f32;
        if a!=0.0 {
            let rad=a.to_radians();
            let (cos,sin)=(rad.cos(),rad.sin());
            // Match rotate_arbitrary's f32 ceil, including its output centre.
            let nw=(w as f32*cos.abs()+h as f32*sin.abs()).ceil() as f64;
            let nh=(h as f32*cos.abs()+w as f32*sin.abs()).ceil() as f64;
            for (x,y) in &mut points {
                let (dx,dy)=(*x-w/2.0,*y-h/2.0);
                (*x,*y)=(dx*cos as f64-dy*sin as f64+nw/2.0,dx*sin as f64+dy*cos as f64+nh/2.0);
            }
            (w,h)=(nw,nh);
        }
        let c=&p["crop"];
        let (cw,ch)=(c["width"].as_f64().unwrap_or(0.0),c["height"].as_f64().unwrap_or(0.0));
        if cw>0.0 && ch>0.0 {
            for (x,y) in &mut points {*x-=c["x"].as_f64().unwrap_or(0.0);*y-=c["y"].as_f64().unwrap_or(0.0);}
            (w,h)=(cw,ch);
        }
        if let (Some(nw),Some(nh))=(p["resize"]["width"].as_f64(),p["resize"]["height"].as_f64()) {
            if nw>0.0 && nh>0.0 {
                for (x,y) in &mut points {*x*=nw/w;*y*=nh/h;}
                (w,h)=(nw,nh);
            }
        }
    }
    let left=points.iter().map(|p|p.0).fold(f64::INFINITY,f64::min).max(0.0);
    let top=points.iter().map(|p|p.1).fold(f64::INFINITY,f64::min).max(0.0);
    let right=points.iter().map(|p|p.0).fold(f64::NEG_INFINITY,f64::max).min(w);
    let bottom=points.iter().map(|p|p.1).fold(f64::NEG_INFINITY,f64::max).min(h);
    (right>left && bottom>top).then_some(([left,top,right-left,bottom-top],[w as u32,h as u32]))
}

pub fn rendered_faces(file_id:i64)->Result<Vec<t_sqlite::Face>,String> {
    let file=t_sqlite::AFile::get_file_info(file_id)?.ok_or("Photo not found")?;
    let path=file.file_path.as_deref().ok_or("Photo has no path")?;
    let (w,h)=t_image::get_image_dimensions(path).or_else(|_|Ok::<_,String>((file.width.unwrap_or(0) as u32,file.height.unwrap_or(0) as u32)))?;
    let orientation=t_image::get_image_orientation(path);
    let conn=t_sqlite::open_conn()?;
    let mut mapped=Vec::new();
    for mut face in t_sqlite::Face::get_for_file(file_id)? {
        let mut bbox:Value=serde_json::from_str(&face.bbox).map_err(|e|e.to_string())?;
        let b=["x","y","width","height"].map(|k|bbox[k].as_f64().unwrap_or(0.0));
        let display=crate::t_face_thumb::is_display_box(&conn,file_id,face.id)?;
        if let Some((rect,dims))=map_bbox(b,w,h,orientation,display,file.edits.as_deref()) {
            for (k,v) in ["x","y","width","height"].into_iter().zip(rect) {bbox[k]=json!(v);}
            bbox["coordinate_space"]=json!("display");bbox["image_width"]=json!(dims[0]);bbox["image_height"]=json!(dims[1]);
            face.bbox=bbox.to_string();mapped.push(face);
        }
    }
    Ok(mapped)
}

// Paths and encoding are export options, never part of a stored recipe.
fn recipe_value(params: t_image::EditParams) -> Result<Value, String> {
    let mut value=serde_json::to_value(params).map_err(|e| e.to_string())?;
    value["sourceFilePath"]=json!(""); value["destFilePath"]=json!("");
    value["outputFormat"]=json!("jpg"); value["quality"]=json!(90);
    if let Some(curves)=value["tools"].get("curves") {
        for name in ["rgb","red","green","blue"] {
            if let Some(points)=curves.get(name) {
                let points:Vec<[f64;2]>=serde_json::from_value(points.clone()).map_err(|e|e.to_string())?;
                crate::t_curves::lut(&points)?;
            }
        }
    }
    value.as_object_mut().unwrap().remove("autoEnhance");
    value.as_object_mut().unwrap().remove("sourceFileId");
    let rotate=value["rotate"].as_i64().unwrap_or(0);
    if rotate % 90 != 0 || value["cropAngle"].as_f64().is_none_or(|a| !a.is_finite() || a.abs()>45.0) {
        return Err("Invalid edit geometry".into());
    }
    Ok(value)
}

enum EditChange { Recipe(Value), Enhance(bool), Reset, Undo, Redo }

fn snapshot(doc: &Value) -> Value {
    json!({"recipe":doc["recipe"],"auto_enhance":doc["auto_enhance"].as_bool().unwrap_or(false),
        "revision":doc["revision"].as_u64().unwrap_or(0),"step":doc["step"].as_str().unwrap_or("Edit")})
}

fn recipe_step<'a>(previous:&Value,next:&'a Value)->&'a str {
    if previous["tools"]!=next["tools"] {
        if let Some(label)=next["tools"]["lastStep"].as_str().filter(|s| !s.is_empty()) {return label;}
    }
    let defaults=json!({"cropAngle":0.0,"crop":{"x":0,"y":0,"width":0,"height":0},"rotate":0,
        "flipHorizontal":false,"flipVertical":false,"redEye":false,"resize":{"width":null,"height":null}});
    for (key,label) in [("cropAngle","Straighten"),("crop","Crop"),("rotate","Rotate"),
        ("flipHorizontal","Flip"),("flipVertical","Flip"),("redEye","Redeye"),("resize","Resize"),
        ("filter","Effect"),("brightness","Fill Light"),("contrast","Contrast"),("saturation","Saturation"),
        ("hue_rotate","Color"),("blur","Soft Focus")] {
        let before=previous.get(key).unwrap_or(&defaults[key]);
        if *before!=next[key] {return label;}
    }
    "Edit"
}

fn change(conn: &mut rusqlite::Connection, file_id: i64, action: EditChange) -> Result<String, String> {
    let tx=conn.transaction().map_err(|e| e.to_string())?;
    let old: Option<String>=tx.query_row("SELECT edits FROM afiles WHERE id=?1",[file_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    let mut doc=state(old.as_deref());
    let mut history=doc["history"].as_array().cloned().unwrap_or_default();
    // Older versions stored recipe-only entries. Upgrade without losing that undo stack.
    if doc["history_version"] != 2 {
        history=history.into_iter().enumerate().map(|(index,recipe)|json!({"recipe":recipe,
            "auto_enhance":doc["auto_enhance"].as_bool().unwrap_or(false),"revision":index,"step":"Edit"})).collect();
    }
    let mut redo=doc["redo"].as_array().cloned().unwrap_or_default();
    let current=snapshot(&doc);
    let mut updated=current.clone();
    let navigation=matches!(action,EditChange::Undo|EditChange::Redo);
    match action {
        EditChange::Recipe(value)=>{
            let label=recipe_step(&current["recipe"],&value).to_owned();
            updated["recipe"]=value;
            updated["step"]=json!(label);
        },
        EditChange::Enhance(enable)=>{updated["auto_enhance"]=json!(enable);updated["step"]=json!("I'm Feeling Lucky");},
        EditChange::Reset=>{updated["recipe"]=Value::Null;updated["auto_enhance"]=json!(false);updated["step"]=json!("Undo All Edits");},
        EditChange::Undo=>{updated=history.pop().ok_or("No edit to undo")?;redo.push(current.clone());},
        EditChange::Redo=>{updated=redo.pop().ok_or("No edit to redo")?;history.push(current.clone());},
    }
    if !navigation {
        if updated["recipe"]==current["recipe"] && updated["auto_enhance"]==current["auto_enhance"] {return Ok(doc.to_string());}
        history.push(current);
        redo.clear();
        let revision=doc["next_revision"].as_u64().unwrap_or(0).max(doc["revision"].as_u64().unwrap_or(0))+1;
        updated["revision"]=json!(revision);
        doc["next_revision"]=json!(revision);
    }
    doc["revision"]=updated["revision"].clone();
    doc["render_key"]=json!(uuid::Uuid::new_v4().to_string());
    doc["recipe"]=updated["recipe"].clone();
    doc["auto_enhance"]=updated["auto_enhance"].clone();
    doc["step"]=updated["step"].clone();
    doc["history"]=json!(history);doc["redo"]=json!(redo);doc["history_version"]=json!(2);
    let text=serde_json::to_string(&doc).map_err(|e|e.to_string())?;
    tx.execute("UPDATE afiles SET edits=?1 WHERE id=?2",rusqlite::params![text,file_id]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e|e.to_string())?;
    Ok(text)
}

pub fn save(file_id: i64, params: t_image::EditParams) -> Result<String, String> {
    let file=t_sqlite::AFile::get_file_info(file_id)?.ok_or("Photo not found")?;
    let path=file.file_path.as_deref().ok_or("Photo has no path")?;
    let mut value=recipe_value(params)?;
    value["orientation"]=json!(t_image::get_image_orientation(path));
    let (w,h)=t_image::get_image_dimensions(path)?;
    validate_geometry(&value,w,h)?;
    let mut conn=t_sqlite::open_conn()?;
    change(&mut conn,file_id,EditChange::Recipe(value))
}

fn validate_geometry(p:&Value,width:u32,height:u32)->Result<(),String> {
    let (mut w,mut h)=(width as f32,height as f32);
    if (5..=8).contains(&p["orientation"].as_i64().unwrap_or(1)) {std::mem::swap(&mut w,&mut h);}
    if p["rotate"].as_i64().unwrap_or(0).rem_euclid(180)==90 {std::mem::swap(&mut w,&mut h);}
    let angle=(p["cropAngle"].as_f64().unwrap_or(0.0) as f32).to_radians();
    (w,h)=((w*angle.cos().abs()+h*angle.sin().abs()).ceil(),(h*angle.cos().abs()+w*angle.sin().abs()).ceil());
    let c=&p["crop"];
    let (cw,ch)=(c["width"].as_u64().unwrap_or(0),c["height"].as_u64().unwrap_or(0));
    if (cw==0)!=(ch==0) || (cw>0 && (c["x"].as_u64().unwrap_or(0).saturating_add(cw)>w as u64
        || c["y"].as_u64().unwrap_or(0).saturating_add(ch)>h as u64)) {return Err("Crop is outside the image".into());}
    Ok(())
}

pub fn restore(file_id: i64, reset: bool) -> Result<String, String> {
    let mut conn=t_sqlite::open_conn()?;
    change(&mut conn,file_id,if reset {EditChange::Reset} else {EditChange::Undo})
}

pub fn redo(file_id:i64)->Result<String,String> {
    let mut conn=t_sqlite::open_conn()?;
    change(&mut conn,file_id,EditChange::Redo)
}

pub fn enhance(file_id:i64,enable:bool)->Result<String,String> {
    let mut conn=t_sqlite::open_conn()?;
    change(&mut conn,file_id,EditChange::Enhance(enable))
}

pub fn render_params(file_id:i64,path:&str,edits:Option<&str>,base_only:bool)->Result<t_image::EditParams,String> {
    let doc=state(edits);
    let mut value=doc["recipe"].clone();
    if value.is_null() || base_only {
        value=json!({"sourceFilePath":"","destFilePath":"","outputFormat":"jpg","orientation":1,
            "flipHorizontal":false,"flipVertical":false,"rotate":0,"crop":{"x":0,"y":0,"width":0,"height":0},
            "resize":{"width":null,"height":null},"redEye":doc["recipe"]["redEye"].as_bool().unwrap_or(false),"tools":doc["recipe"]["tools"]});
    }
    value["sourceFilePath"]=json!(path);
    // The editor draws its live point-curve draft over this uncurved base.
    // Final viewer/thumbnail/face/export recipes retain all four curves.
    if base_only {
        if let Some(tools)=value["tools"].as_object_mut() { tools.remove("curves"); }
    }
    value["orientation"]=json!(t_image::get_image_orientation(path));
    value["sourceFileId"]=json!(file_id);
    value["autoEnhance"]=json!(doc["auto_enhance"].as_bool().unwrap_or(false));
    serde_json::from_value(value).map_err(|e|e.to_string())
}

pub async fn render(file_id: i64, thumbnail: bool) -> Result<Vec<u8>, String> {
    render_mode(file_id,thumbnail,false).await
}

pub async fn render_base(file_id:i64)->Result<Vec<u8>,String> {render_mode(file_id,false,true).await}

async fn render_mode(file_id: i64, thumbnail: bool, base_only:bool) -> Result<Vec<u8>, String> {
    let file=t_sqlite::AFile::get_file_info(file_id)?.ok_or("Photo not found")?;
    let doc=state(file.edits.as_deref());
    let key=format!("edit-v2:{}:{file_id}:{}:{}:{thumbnail}:{base_only}",t_image::ORIENTATION_ALGORITHM_VERSION,doc["render_key"].as_str().unwrap_or(""),file.modified_at.unwrap_or(0));
    if let Some(data)=crate::t_thumb_cache::get_face(&key) {return Ok(data);}
    let _permit=RENDERING.get_or_init(||tokio::sync::Semaphore::new(2)).acquire().await.map_err(|e|e.to_string())?;
    if let Some(data)=crate::t_thumb_cache::get_face(&key) {return Ok(data);}
    let params=render_params(file_id,file.file_path.as_deref().ok_or("Photo has no path")?,file.edits.as_deref(),base_only)?;
    let img=t_image::get_edited_image(&params).await?;
    let data=tauri::async_runtime::spawn_blocking(move || {
        let img=if thumbnail {img.thumbnail(512,512)} else {img};
        crate::t_jpeg::encode_rgb8(&img.to_rgb8(),90)
    }).await.map_err(|e|e.to_string())??;
    crate::t_thumb_cache::put_face(&key,&data)?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unedited_rotation_is_named_rotate_not_straighten() {
        let next=json!({"cropAngle":0.0,"crop":{"x":0,"y":0,"width":0,"height":0},"rotate":90});
        assert_eq!(recipe_step(&Value::Null,&next),"Rotate");
    }
    #[test]
    fn face_mapping_matches_renderer_for_all_exif_orientations_and_straighten() {
        use image::{DynamicImage,GenericImageView};
        let mut source=image::RgbImage::new(80,60);
        for y in 12..24 {for x in 20..36 {source.put_pixel(x,y,image::Rgb([255,0,0]));}}
        for orientation in 1..=8 {for angle in [0.0,9.0] {
            let p=json!({"sourceFilePath":"","destFilePath":"","outputFormat":"jpg","orientation":orientation,
                "flipHorizontal":true,"flipVertical":false,"rotate":90,"cropAngle":angle,
                "crop":{"x":0,"y":0,"width":0,"height":0},"resize":{"width":null,"height":null}});
            let params=serde_json::from_value(p.clone()).unwrap();
            let actual=t_image::apply_edit_params(t_image::apply_orientation(DynamicImage::ImageRgb8(source.clone()),orientation),&params).unwrap();
            let doc=json!({"recipe":p}).to_string();
            let (mapped,dims)=map_bbox([20.0,12.0,16.0,12.0],80,60,orientation,false,Some(&doc)).unwrap();
            assert_eq!(actual.dimensions(),(dims[0],dims[1]));
            let mut pixels=Vec::new();
            for (x,y,pixel) in actual.to_rgb8().enumerate_pixels() {if pixel[0]>128 {pixels.push((x as f64+0.5,y as f64+0.5));}}
            let cx=pixels.iter().map(|p|p.0).sum::<f64>()/pixels.len() as f64;
            let cy=pixels.iter().map(|p|p.1).sum::<f64>()/pixels.len() as f64;
            assert!((cx-mapped[0]-mapped[2]/2.0).abs()<0.7,"orientation {orientation}, angle {angle}");
            assert!((cy-mapped[1]-mapped[3]/2.0).abs()<0.7,"orientation {orientation}, angle {angle}");
        }}
    }

    #[test]
    fn crop_resize_clip_faces_and_reject_invalid_crops() {
        let p=json!({"orientation":1,"rotate":90,"cropAngle":0,"crop":{"x":5,"y":4,"width":50,"height":70},"resize":{"width":100,"height":140}});
        validate_geometry(&p,80,60).unwrap();
        let doc=json!({"recipe":p}).to_string();
        assert_eq!(map_bbox([20.0,12.0,16.0,12.0],80,60,1,false,Some(&doc)),Some(([62.0,32.0,24.0,32.0],[100,140])));
        assert!(map_bbox([100.0,100.0,5.0,5.0],80,60,1,false,Some(&doc)).is_none());
        assert!(validate_geometry(&json!({"crop":{"x":79,"y":0,"width":2,"height":2}}),80,60).is_err());
    }
    #[test]
    fn save_undo_reset_preserve_other_edits_and_original() {
        let mut conn=rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE afiles(id INTEGER PRIMARY KEY,edits TEXT);INSERT INTO afiles VALUES(1,'{\"auto_enhance\":true}');").unwrap();
        let first=change(&mut conn,1,EditChange::Recipe(json!({"rotate":90}))).unwrap();
        assert_eq!(state(Some(&first))["recipe"]["rotate"],90);
        let second=change(&mut conn,1,EditChange::Recipe(json!({"rotate":180}))).unwrap();
        assert_eq!(state(Some(&second))["history"].as_array().unwrap().len(),2);
        let undo=change(&mut conn,1,EditChange::Undo).unwrap();
        assert_eq!(state(Some(&undo))["recipe"]["rotate"],90);
        let reset=change(&mut conn,1,EditChange::Reset).unwrap();
        assert!(state(Some(&reset))["recipe"].is_null());
        assert_eq!(state(Some(&reset))["auto_enhance"],false);
        let undo_reset=change(&mut conn,1,EditChange::Undo).unwrap();
        assert_eq!(state(Some(&undo_reset))["recipe"]["rotate"],90);
    }

    #[test]
    fn wand_and_geometry_share_persisted_history_and_branching_redo() {
        let mut conn=rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE afiles(id INTEGER PRIMARY KEY,edits TEXT);INSERT INTO afiles VALUES(1,NULL);").unwrap();
        let wand=change(&mut conn,1,EditChange::Enhance(true)).unwrap();
        let undo=change(&mut conn,1,EditChange::Undo).unwrap();
        assert_eq!(state(Some(&undo))["revision"],0);
        assert_eq!(state(Some(&undo))["auto_enhance"],false);
        let redo=change(&mut conn,1,EditChange::Redo).unwrap();
        assert_eq!(state(Some(&redo))["revision"],state(Some(&wand))["revision"]);
        assert_eq!(state(Some(&redo))["auto_enhance"],true);
        change(&mut conn,1,EditChange::Recipe(json!({"rotate":90}))).unwrap();
        change(&mut conn,1,EditChange::Undo).unwrap();
        let branched=change(&mut conn,1,EditChange::Recipe(json!({"rotate":180}))).unwrap();
        assert!(state(Some(&branched))["redo"].as_array().unwrap().is_empty());
        assert_eq!(state(Some(&branched))["revision"],3);
        // An unchanged Save does not create another undo step.
        let unchanged=change(&mut conn,1,EditChange::Recipe(json!({"rotate":180}))).unwrap();
        assert_eq!(unchanged,branched);
    }
}
