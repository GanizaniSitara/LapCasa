//! Persist edits independently of original bytes; shared by display and export.
use serde_json::{json, Value};
use crate::{t_image, t_sqlite};
use std::sync::OnceLock;

static RENDERING: OnceLock<tokio::sync::Semaphore> = OnceLock::new();

pub fn state(edits: Option<&str>) -> Value {
    let mut doc=edits.and_then(|s| serde_json::from_str(s).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}));
    if let Some(recipe)=doc.get_mut("recipe") { canonical_tools(recipe); }
    for key in ["history","redo"] {
        if let Some(entries)=doc.get_mut(key).and_then(Value::as_array_mut) {
            for entry in entries {
                if entry.get("recipe").is_some() { canonical_tools(&mut entry["recipe"]); }
                else { canonical_tools(entry); }
            }
        }
    }
    doc
}

// The previous public source used "tools"; keep those edits and undo snapshots readable.
fn canonical_tools(recipe: &mut Value) {
    if let Some(recipe)=recipe.as_object_mut() {
        if let Some(tools)=recipe.remove("tools") { recipe.entry("picasa").or_insert(tools); }
    }
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
    if let Some(curves)=value["picasa"].get("curves") {
        for name in ["rgb","red","green","blue"] {
            if let Some(points)=curves.get(name) {
                let points:Vec<[f64;2]>=serde_json::from_value(points.clone()).map_err(|e|e.to_string())?;
                crate::t_curves::lut(&points)?;
            }
        }
    }
    if let Some(local)=value["picasa"].get("local") { crate::t_edit_local::validate(local)?; }
    value.as_object_mut().unwrap().remove("autoEnhance");
    value.as_object_mut().unwrap().remove("autoEnhancePreset");
    value.as_object_mut().unwrap().remove("sourceFileId");
    let rotate=value["rotate"].as_i64().unwrap_or(0);
    if rotate % 90 != 0 || value["cropAngle"].as_f64().is_none_or(|a| !a.is_finite() || a.abs()>45.0) {
        return Err("Invalid edit geometry".into());
    }
    Ok(value)
}

enum EditChange { Recipe(Value), Enhance(bool), Reset, Undo, Redo, Paste(Value, Option<u8>), Batch(Value, &'static str), Lucky, Style(u8) }

fn snapshot(doc: &Value) -> Value {
    let auto_enhance = doc["auto_enhance"].as_bool().unwrap_or(false);
    let preset = doc["auto_enhance_preset"]
        .as_u64()
        .map(|v| crate::t_enhance::normalize_preset(v as u8))
        .unwrap_or(if auto_enhance { 1 } else { 0 });
    json!({
        "recipe": doc["recipe"],
        "auto_enhance": auto_enhance,
        "auto_enhance_preset": preset,
        "revision": doc["revision"].as_u64().unwrap_or(0),
        "step": doc["step"].as_str().unwrap_or("Edit")
    })
}

fn recipe_step<'a>(previous:&Value,next:&'a Value)->&'a str {
    if previous["picasa"]!=next["picasa"] {
        if let Some(label)=next["picasa"]["lastStep"].as_str().filter(|s| !s.is_empty()) {return label;}
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
    // LAP-012 stored recipe-only entries. Upgrade without losing that undo stack.
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
        EditChange::Enhance(enable)=>{
            let (next_enable, next_preset) = if enable {
                if current["auto_enhance"].as_bool().unwrap_or(false) {
                    let cur_p = crate::t_enhance::normalize_preset(
                        current["auto_enhance_preset"].as_u64().unwrap_or(1) as u8,
                    );
                    if cur_p >= crate::t_enhance::PRESET_COUNT {
                        (false, 0)
                    } else {
                        (true, cur_p + 1)
                    }
                } else {
                    (true, 1)
                }
            } else {
                (false, 0)
            };
            updated["auto_enhance"]=json!(next_enable);
            updated["auto_enhance_preset"]=json!(next_preset);
            updated["step"]=json!("I'm Feeling Lucky");
        },
        EditChange::Reset=>{
            updated["recipe"]=Value::Null;
            updated["auto_enhance"]=json!(false);
            updated["auto_enhance_preset"]=json!(0);
            updated["step"]=json!("Undo All Edits");
        },
        EditChange::Paste(value,preset)=>{
            updated["recipe"]=value;
            // The copied photo's I'm Feeling Lucky style comes with it; without one the target keeps its own.
            if let Some(p)=preset {updated["auto_enhance"]=json!(true);updated["auto_enhance_preset"]=json!(p);}
            updated["step"]=json!("Paste All Effects");
        },
        EditChange::Batch(value,step)=>{
            updated["recipe"]=value;
            updated["step"]=json!(step);
        },
        // Batch Edit's I'm Feeling Lucky: always Style 1, never the editor's click-to-cycle (LAP-085).
        EditChange::Lucky=>{
            updated["auto_enhance"]=json!(true);
            updated["auto_enhance_preset"]=json!(1);
            updated["step"]=json!("I'm Feeling Lucky");
        },
        // The editor's Orig / 1-5 buttons (LAP-100): that style directly, 0 = Original.
        EditChange::Style(style)=>{
            let style=style.min(crate::t_enhance::PRESET_COUNT);
            if style>0 || current["auto_enhance"].as_bool().unwrap_or(false) {
                updated["auto_enhance"]=json!(style>0);
                updated["auto_enhance_preset"]=json!(style);
                updated["step"]=json!("I'm Feeling Lucky");
            }
        },
        EditChange::Undo=>{updated=history.pop().ok_or("No edit to undo")?;redo.push(current.clone());},
        EditChange::Redo=>{updated=redo.pop().ok_or("No edit to redo")?;history.push(current.clone());},
    }
    if !navigation {
        if updated["recipe"]==current["recipe"]
            && updated["auto_enhance"]==current["auto_enhance"]
            && updated["auto_enhance_preset"]==current["auto_enhance_preset"]
        {
            return Ok(doc.to_string());
        }
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
    doc["auto_enhance_preset"]=updated["auto_enhance_preset"].clone();
    doc["step"]=updated["step"].clone();
    doc["history"]=json!(history);doc["redo"]=json!(redo);doc["history_version"]=json!(2);
    let text=serde_json::to_string(&doc).map_err(|e|e.to_string())?;
    tx.execute("UPDATE afiles SET edits=?1 WHERE id=?2",rusqlite::params![text,file_id]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e|e.to_string())?;
    crate::native_grid::refresh_file(file_id);
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

pub fn lucky_style(file_id:i64,style:u8)->Result<String,String> {
    let mut conn=t_sqlite::open_conn()?;
    change(&mut conn,file_id,EditChange::Style(style))
}

pub fn effects_from_doc(doc: &Value) -> Value {
    let auto_enhance = doc["auto_enhance"].as_bool().unwrap_or(false);
    // LAP-085: which I'm Feeling Lucky style (1-5), so a paste reproduces the same look; 0 = none.
    let preset = if auto_enhance {
        doc["auto_enhance_preset"].as_u64().map(|v| crate::t_enhance::normalize_preset(v as u8)).unwrap_or(1)
    } else { 0 };
    let recipe = &doc["recipe"];
    let picasa = recipe.get("picasa").and_then(|p| p.as_object()).map(|p| {
        let mut obj = p.clone();
        obj.remove("retouch");
        obj.remove("lastStep");
        // Automask (LAP-107): a region's mask belongs to its photo; whole-photo ones copy.
        if let Some(local) = obj.get_mut("local").and_then(Value::as_array_mut) { local.retain(|a| a["mask"].as_str().is_none()); }
        Value::Object(obj)
    }).unwrap_or(Value::Null);

    json!({
        "cropAngle": recipe.get("cropAngle").and_then(Value::as_f64).unwrap_or(0.0),
        "filter": recipe.get("filter").cloned().unwrap_or(Value::Null),
        "brightness": recipe.get("brightness").cloned().unwrap_or(Value::Null),
        "contrast": recipe.get("contrast").cloned().unwrap_or(Value::Null),
        "blur": recipe.get("blur").cloned().unwrap_or(Value::Null),
        "hue_rotate": recipe.get("hue_rotate").cloned().unwrap_or(Value::Null),
        "saturation": recipe.get("saturation").cloned().unwrap_or(Value::Null),
        "autoEnhance": auto_enhance,
        "autoEnhancePreset": preset,
        "picasa": picasa,
    })
}

pub fn copy_effects(file_id: i64) -> Result<Value, String> {
    let file = t_sqlite::AFile::get_file_info(file_id)?.ok_or("Photo not found")?;
    let doc = state(file.edits.as_deref());
    Ok(effects_from_doc(&doc))
}

pub fn init_recipe(base: Option<&Value>, orientation: i32) -> Value {
    let default_recipe = json!({
        "sourceFilePath": "", "destFilePath": "", "outputFormat": "jpg",
        "orientation": 1, "flipHorizontal": false, "flipVertical": false, "rotate": 0,
        "cropAngle": 0.0, "crop": {"x": 0, "y": 0, "width": 0, "height": 0},
        "resize": {"width": null, "height": null}, "redEye": false, "picasa": {}
    });
    let mut recipe = match base {
        Some(b) if b.is_object() => b.clone(),
        _ => default_recipe,
    };
    recipe["sourceFilePath"] = json!("");
    recipe["destFilePath"] = json!("");
    recipe["outputFormat"] = json!("jpg");
    recipe["orientation"] = json!(orientation);
    if !recipe.get("picasa").map_or(false, Value::is_object) {
        recipe["picasa"] = json!({});
    }
    recipe
}

pub fn merge_effects(base: Option<&Value>, orientation: i32, effects: &Value) -> Value {
    let mut recipe = init_recipe(base, orientation);

    if let Some(angle) = effects["cropAngle"].as_f64() {
        if angle != 0.0 { recipe["cropAngle"] = json!(angle); }
    }
    if let Some(filter) = effects["filter"].as_str() {
        if !filter.is_empty() { recipe["filter"] = json!(filter); }
    }
    if let Some(b) = effects["brightness"].as_i64() {
        if b != 0 { recipe["brightness"] = json!(b); }
    }
    if let Some(c) = effects["contrast"].as_f64() {
        if c != 0.0 { recipe["contrast"] = json!(c); }
    }
    if let Some(bl) = effects["blur"].as_f64() {
        if bl > 0.0 { recipe["blur"] = json!(bl); }
    }
    if let Some(h_rot) = effects["hue_rotate"].as_i64() {
        if h_rot != 0 { recipe["hue_rotate"] = json!(h_rot); }
    }
    if let Some(s) = effects["saturation"].as_f64() {
        if (s - 1.0).abs() > 1e-5 { recipe["saturation"] = json!(s); }
    }

    if let Some(p_eff) = effects.get("picasa").and_then(|p| p.as_object()) {
        for key in ["fillLight", "highlights", "shadows", "temperature"] {
            if let Some(v) = p_eff.get(key).and_then(Value::as_f64) {
                if v != 0.0 { recipe["picasa"][key] = json!(v); }
            }
        }
        if let Some(neutral) = p_eff.get("neutral") {
            if !neutral.is_null() { recipe["picasa"]["neutral"] = neutral.clone(); }
        }
        if let Some(curves) = p_eff.get("curves") {
            if !curves.is_null() { recipe["picasa"]["curves"] = curves.clone(); }
        }
        if let Some(eff_arr) = p_eff.get("effects").and_then(Value::as_array) {
            if !recipe["picasa"].get("effects").map_or(false, Value::is_array) {
                recipe["picasa"]["effects"] = json!([]);
            }
            if let Some(target_arr) = recipe["picasa"]["effects"].as_array_mut() {
                for item in eff_arr {
                    target_arr.push(item.clone());
                }
            }
        }
        if let Some(text_arr) = p_eff.get("text").and_then(Value::as_array) {
            if !recipe["picasa"].get("text").map_or(false, Value::is_array) {
                recipe["picasa"]["text"] = json!([]);
            }
            if let Some(target_arr) = recipe["picasa"]["text"].as_array_mut() {
                for item in text_arr {
                    target_arr.push(item.clone());
                }
            }
        }
    }

    recipe["picasa"]["lastStep"] = json!("Paste All Effects");
    recipe
}

pub fn paste_effects_on(
    conn: &mut rusqlite::Connection,
    files: &[(i64, Option<String>, u32, u32, i32)],
    effects: &Value,
) -> Value {
    let mut ok = Vec::new();
    let mut failed = Vec::new();
    // The copied photo's style, if it had I'm Feeling Lucky on (older clipboards: Style 1).
    let preset = effects["autoEnhance"].as_bool().unwrap_or(false).then(|| {
        crate::t_enhance::normalize_preset(effects["autoEnhancePreset"].as_u64().unwrap_or(1) as u8)
    });

    for &(id, ref edits, w, h, orientation) in files {
        let doc = state(edits.as_deref());
        let recipe = merge_effects(doc.get("recipe"), orientation, effects);

        if let Err(e) = validate_geometry(&recipe, w, h) {
            failed.push(json!({"id": id, "error": e}));
            continue;
        }

        match change(conn, id, EditChange::Paste(recipe, preset)) {
            Ok(_) => ok.push(id),
            Err(e) => failed.push(json!({"id": id, "error": e})),
        }
    }

    json!({"ok": ok, "failed": failed})
}

pub fn paste_effects(file_ids: &[i64], effects: &Value) -> Result<Value, String> {
    let mut conn = t_sqlite::open_conn()?;
    let mut files = Vec::new();
    let mut failed = Vec::new();

    for &id in file_ids {
        let file = match t_sqlite::AFile::get_file_info(id) {
            Ok(Some(f)) => f,
            Ok(None) => { failed.push(json!({"id": id, "error": "Photo not found"})); continue; },
            Err(e) => { failed.push(json!({"id": id, "error": e})); continue; },
        };
        let path = match file.file_path.as_deref() {
            Some(p) if !p.is_empty() => p,
            _ => { failed.push(json!({"id": id, "error": "Photo has no path"})); continue; },
        };
        if file.file_type.unwrap_or(0) == 2 {
            failed.push(json!({"id": id, "error": "Not an image"}));
            continue;
        }
        let (w, h) = match t_image::get_image_dimensions(path) {
            Ok(dims) => dims,
            Err(e) => { failed.push(json!({"id": id, "error": format!("Not an image: {}", e)})); continue; },
        };
        let orientation = t_image::get_image_orientation(path);
        files.push((id, file.edits, w, h, orientation));
    }

    let mut res = paste_effects_on(&mut conn, &files, effects);
    if !failed.is_empty() {
        if let Some(failed_arr) = res.get_mut("failed").and_then(Value::as_array_mut) {
            failed.extend(failed_arr.drain(..));
            *failed_arr = failed;
        }
    }

    Ok(res)
}

fn load_small_image(path: &str, file_type: i64, orientation: i32, size: u32) -> Result<image::DynamicImage, String> {
    if file_type == 3 {
        if let Ok(Some(bytes)) = t_image::get_raw_thumbnail(path, orientation, size, true) {
            if let Ok(img) = image::load_from_memory(&bytes) {
                return Ok(img);
            }
        }
    }
    if let Ok(Some(bytes)) = t_image::get_image_thumbnail(path, orientation, size) {
        if let Ok(img) = image::load_from_memory(&bytes) {
            return Ok(img);
        }
    }
    let img = image::open(path).map_err(|e| e.to_string())?;
    Ok(t_image::apply_orientation(img, orientation))
}

// Percentile levels are bounded even for dark, monochrome and nearly flat photos.
fn compute_auto_levels(img: &image::DynamicImage, color: bool) -> [[f64; 2]; 3] {
    let rgb = img.thumbnail(256, 256).to_rgb8();
    let mut hist = [[0u32; 256]; 3];
    for p in rgb.pixels() {
        let y = (0.2126*p[0] as f64 + 0.7152*p[1] as f64 + 0.0722*p[2] as f64).round() as usize;
        for c in 0..3 { hist[c][if color { p[c] as usize } else { y }] += 1; }
    }
    let count = rgb.width() as f64 * rgb.height() as f64;
    std::array::from_fn(|c| {
        if count == 0.0 { return [0.0, 1.0]; }
        let percentile = |fraction: f64| {
            let mut n = 0u32;
            for (i, bins) in hist[c].iter().enumerate() {
                n += bins;
                if n as f64 >= (count * fraction).max(1.0) { return i as f64; }
            }
            255.0
        };
        let (low, high) = (percentile(0.005), percentile(0.995));
        if high-low < 8.0 { return [0.0, 1.0]; }
        // Limit both the black-point shift and gain: never amplify a missing channel.
        [low.min(if color { 4.0 } else { 12.0 }),
            (255.0/(high-low)).clamp(1.0, if color { 1.08 } else { 1.25 })]
    })
}

pub fn tool_recipe(mut recipe: Value, op: &str) -> Result<(Value, &'static str), String> {
    let name: &'static str = match op {
        "sharpen" => "Sharpen",
        "sepia" => "Sepia",
        "bw" => "B&W",
        "warmify" => "Warmify",
        "film_grain" => "Film Grain",
        _ => return Err(format!("Unknown batch edit tool: {}", op)),
    };
    if !recipe.get("picasa").map_or(false, Value::is_object) {
        recipe["picasa"] = json!({});
    }
    if !recipe["picasa"].get("effects").map_or(false, Value::is_array) {
        recipe["picasa"]["effects"] = json!([]);
    }
    if let Some(arr) = recipe["picasa"]["effects"].as_array_mut() {
        arr.push(json!({"name": name, "amount": 0.5}));
    }
    recipe["picasa"]["lastStep"] = json!(name);
    Ok((recipe, name))
}

pub fn batch_edit(file_ids: &[i64], op: &str) -> Result<Value, String> {
    match op {
        "lucky" | "undo_all" | "auto_contrast" | "auto_color"
        | "sharpen" | "sepia" | "bw" | "warmify" | "film_grain" => {},
        _ => return Err(format!("Unknown batch edit operation: {}", op)),
    }

    let mut conn = t_sqlite::open_conn()?;
    let mut ok = Vec::new();
    let mut failed = Vec::new();

    for &id in file_ids {
        let file = match t_sqlite::AFile::get_file_info(id) {
            Ok(Some(f)) => f,
            Ok(None) => { failed.push(json!({"id": id, "error": "Photo not found"})); continue; },
            Err(e) => { failed.push(json!({"id": id, "error": e})); continue; },
        };
        let path = match file.file_path.as_deref() {
            Some(p) if !p.is_empty() => p,
            _ => { failed.push(json!({"id": id, "error": "Photo has no path"})); continue; },
        };
        if file.file_type.unwrap_or(0) == 2 {
            failed.push(json!({"id": id, "error": "Not an image"}));
            continue;
        }

        if op == "lucky" {
            match change(&mut conn, id, EditChange::Lucky) {
                Ok(_) => ok.push(id),
                Err(e) => failed.push(json!({"id": id, "error": e})),
            }
            continue;
        }

        if op == "undo_all" {
            match change(&mut conn, id, EditChange::Reset) {
                Ok(_) => ok.push(id),
                Err(e) => failed.push(json!({"id": id, "error": e})),
            }
            continue;
        }

        let (w, h) = match t_image::get_image_dimensions(path) {
            Ok(dims) => dims,
            Err(e) => { failed.push(json!({"id": id, "error": format!("Not an image: {}", e)})); continue; },
        };
        let orientation = t_image::get_image_orientation(path);
        let file_type = file.file_type.unwrap_or(0);

        let doc = state(file.edits.as_deref());
        let mut recipe = init_recipe(doc.get("recipe"), orientation);

        let step_label = if matches!(op, "sharpen" | "sepia" | "bw" | "warmify" | "film_grain") {
            match tool_recipe(recipe, op) {
                Ok((r, label)) => {
                    recipe = r;
                    label
                },
                Err(e) => { failed.push(json!({"id": id, "error": e})); continue; },
            }
        } else if op == "auto_contrast" {
            let img = match load_small_image(path, file_type, orientation, 256) {
                Ok(im) => im,
                Err(e) => { failed.push(json!({"id": id, "error": format!("Failed to decode image: {}", e)})); continue; },
            };
            recipe["brightness"] = json!(0);
            recipe["contrast"] = json!(0);
            recipe["picasa"]["autoContrast"] = json!(compute_auto_levels(&img, false));
            recipe["picasa"]["lastStep"] = json!("Auto Contrast");
            "Auto Contrast"
        } else if op == "auto_color" {
            let img = match load_small_image(path, file_type, orientation, 96) {
                Ok(im) => im,
                Err(e) => { failed.push(json!({"id": id, "error": format!("Failed to decode image: {}", e)})); continue; },
            };
            recipe["picasa"]["neutral"] = Value::Null;
            recipe["picasa"]["autoColor"] = json!(compute_auto_levels(&img, true));
            recipe["picasa"]["lastStep"] = json!("Auto Color");
            "Auto Color"
        } else {
            "Edit"
        };

        if let Err(e) = validate_geometry(&recipe, w, h) {
            failed.push(json!({"id": id, "error": e}));
            continue;
        }

        match change(&mut conn, id, EditChange::Batch(recipe, step_label)) {
            Ok(_) => ok.push(id),
            Err(e) => failed.push(json!({"id": id, "error": e})),
        }
    }

    Ok(json!({"ok": ok, "failed": failed}))
}

pub fn render_params(file_id:i64,path:&str,edits:Option<&str>,base_only:bool)->Result<t_image::EditParams,String> {
    let doc=state(edits);
    let mut value=doc["recipe"].clone();
    if value.is_null() || base_only {
        value=json!({"sourceFilePath":"","destFilePath":"","outputFormat":"jpg","orientation":1,
            "flipHorizontal":false,"flipVertical":false,"rotate":0,"crop":{"x":0,"y":0,"width":0,"height":0},
            "resize":{"width":null,"height":null},"redEye":doc["recipe"]["redEye"].as_bool().unwrap_or(false),"picasa":doc["recipe"]["picasa"]});
    }
    value["sourceFilePath"]=json!(path);
    // The editor draws its live point-curve draft over this uncurved base.
    // Final viewer/thumbnail/face/export recipes retain all four curves.
    if base_only {
        if let Some(tools)=value["picasa"].as_object_mut() { tools.remove("curves"); }
    }
    value["orientation"]=json!(t_image::get_image_orientation(path));
    value["sourceFileId"]=json!(file_id);
    value["autoEnhance"]=json!(doc["auto_enhance"].as_bool().unwrap_or(false));
    value["autoEnhancePreset"]=json!(doc["auto_enhance_preset"].as_u64().map(|v| crate::t_enhance::normalize_preset(v as u8)).unwrap_or(1));
    serde_json::from_value(value).map_err(|e|e.to_string())
}

pub async fn render(file_id: i64, thumbnail: bool) -> Result<Vec<u8>, String> {
    render_mode(file_id, thumbnail, false, false).await
}

pub async fn render_full(file_id: i64) -> Result<Vec<u8>, String> {
    render_mode(file_id, false, false, true).await
}

pub async fn render_base(file_id: i64) -> Result<Vec<u8>, String> {
    render_mode(file_id, false, true, false).await
}

fn has_base_recipe_tools(doc: &Value) -> bool {
    let r = &doc["recipe"];
    if !r.is_object() { return false; }
    if r["redEye"].as_bool().unwrap_or(false) { return true; }
    let p = &r["picasa"];
    if !p.is_object() { return false; }
    p["autoContrast"].is_array() || p["autoColor"].is_array()
        || p["fillLight"].as_f64().unwrap_or(0.0) != 0.0
        || p["highlights"].as_f64().unwrap_or(0.0) != 0.0
        || p["shadows"].as_f64().unwrap_or(0.0) != 0.0
        || p["temperature"].as_f64().unwrap_or(0.0) != 0.0
        || p["neutral"].as_array().is_some_and(|a| !a.is_empty())
        || p["effects"].as_array().is_some_and(|a| !a.is_empty())
        || p["retouch"].as_array().is_some_and(|a| !a.is_empty())
        || p["text"].as_array().is_some_and(|a| !a.is_empty())
        || p["local"].as_array().is_some_and(|a| !a.is_empty())
}

pub(crate) fn render_thumbnail_sync(file_id: i64) -> Option<Vec<u8>> {
    let file = t_sqlite::AFile::get_file_info(file_id).ok().flatten()?;
    let doc = state(file.edits.as_deref());
    let key = format!("edit-v3:{}:{file_id}:{}:{}:true:false:false",
        t_image::ORIENTATION_ALGORITHM_VERSION,
        doc["render_key"].as_str().unwrap_or(""),
        file.modified_at.unwrap_or(0));
    if let Some(data) = crate::t_thumb_cache::get_face(&key) { return Some(data); }
    let path = file.file_path.as_deref()?;
    let orientation = file.e_orientation.unwrap_or(1) as i32;
    let preview = crate::t_thumb_cache::get_preview(file_id)
        .or_else(|| tauri::async_runtime::block_on(crate::t_image::get_or_create_preview_image(file_id, path, orientation)).ok())?;
    let (raw_w, raw_h) = t_image::get_image_dimensions(path).unwrap_or((2048, 2048));
    let params = render_params(file_id, path, file.edits.as_deref(), false).ok()?;
    let mut img = image::load_from_memory(&preview).ok()?;
    let scaled_params = params.scaled_to_preview(raw_w, raw_h, img.width(), img.height());
    img = t_image::apply_edit_params(img, &scaled_params).ok()?;
    let thumb_img = img.thumbnail(512, 512);
    let encoded = crate::t_jpeg::encode_rgb8(&thumb_img.to_rgb8(), 90).ok()?;
    let _ = crate::t_thumb_cache::put_face(&key, &encoded);
    Some(encoded)
}

async fn render_mode(file_id: i64, thumbnail: bool, base_only: bool, full_res: bool) -> Result<Vec<u8>, String> {
    let file = t_sqlite::AFile::get_file_info(file_id)?.ok_or("Photo not found")?;
    let doc = state(file.edits.as_deref());
    let key = format!("edit-v3:{}:{file_id}:{}:{}:{thumbnail}:{base_only}:{full_res}",
        t_image::ORIENTATION_ALGORITHM_VERSION,
        doc["render_key"].as_str().unwrap_or(""),
        file.modified_at.unwrap_or(0));
    if let Some(data) = crate::t_thumb_cache::get_face(&key) { return Ok(data); }
    let _permit = RENDERING.get_or_init(|| tokio::sync::Semaphore::new(2)).acquire().await.map_err(|e| e.to_string())?;
    if let Some(data) = crate::t_thumb_cache::get_face(&key) { return Ok(data); }
    let path = file.file_path.as_deref().ok_or("Photo has no path")?;
    let orientation = file.e_orientation.unwrap_or(1) as i32;

    let data = if base_only && !has_base_recipe_tools(&doc) {
        let preview = crate::t_image::get_or_create_preview_image(file_id, path, orientation).await?;
        if doc["auto_enhance"].as_bool().unwrap_or(false) {
            crate::t_enhance::apply_auto_enhance_to_bytes_for_file(&preview, Some(file_id)).unwrap_or(preview)
        } else {
            preview
        }
    } else if thumbnail || !full_res {
        let preview = crate::t_image::get_or_create_preview_image(file_id, path, orientation).await?;
        let (raw_w, raw_h) = t_image::get_image_dimensions(path).unwrap_or((2048, 2048));
        let params = render_params(file_id, path, file.edits.as_deref(), base_only)?;
        let encoded = tauri::async_runtime::spawn_blocking(move || {
            let mut img = image::load_from_memory(&preview)
                .map_err(|e| format!("Failed to decode preview buffer: {}", e))?;
            let scaled_params = params.scaled_to_preview(raw_w, raw_h, img.width(), img.height());
            img = t_image::apply_edit_params(img, &scaled_params)?;
            if thumbnail {
                let thumb_img = img.thumbnail(512, 512);
                crate::t_jpeg::encode_rgb8(&thumb_img.to_rgb8(), 90)
            } else {
                crate::t_jpeg::encode_rgb8(&img.to_rgb8(), 88)
            }
        }).await.map_err(|e| e.to_string())??;
        encoded
    } else {
        let params = render_params(file_id, path, file.edits.as_deref(), false)?;
        let img = t_image::get_edited_image(&params).await?;
        let encoded = tauri::async_runtime::spawn_blocking(move || {
            crate::t_jpeg::encode_rgb8(&img.to_rgb8(), 90)
        }).await.map_err(|e| e.to_string())??;
        encoded
    };
    crate::t_thumb_cache::put_face(&key, &data)?;
    Ok(data)
}

/// The photo as LapCasa shows it, at full resolution, for handing it out (Send to > Email,
/// LAP-008): its edits rendered, or its view rotation applied when that is all it has. None when
/// it is unedited, so the original file is the photo as shown.
pub async fn render_for_export(file_id: i64) -> Result<Option<Vec<u8>>, String> {
    let file = t_sqlite::AFile::get_file_info(file_id)?.ok_or("Photo not found")?;
    let doc = state(file.edits.as_deref());
    if doc["recipe"].is_object() || doc["auto_enhance"].as_bool().unwrap_or(false) {
        return render_full(file_id).await.map(Some);
    }
    let rotate = file.rotate.unwrap_or(0).rem_euclid(360);
    if rotate == 0 { return Ok(None); }
    let path = file.file_path.as_deref().ok_or("Photo has no path")?;
    let edits = json!({"recipe": {"sourceFilePath":"","destFilePath":"","outputFormat":"jpg","orientation":1,
        "flipHorizontal":false,"flipVertical":false,"rotate":rotate,"crop":{"x":0,"y":0,"width":0,"height":0},
        "resize":{"width":null,"height":null}}}).to_string();
    let params = render_params(file_id, path, Some(&edits), false)?;
    let img = t_image::get_edited_image(&params).await?;
    tauri::async_runtime::spawn_blocking(move || crate::t_jpeg::encode_rgb8(&img.to_rgb8(), 90))
        .await.map_err(|e| e.to_string())?.map(Some)
}

/// The photo as LapCasa shows it, decoded at full resolution (Export Picture to Folder and Make a
/// Poster, LAP-049): its saved edits and auto-enhance rendered, or else its view rotation applied.
pub async fn image_as_shown(file_id: i64) -> Result<image::DynamicImage, String> {
    let file = t_sqlite::AFile::get_file_info(file_id)?.ok_or("Photo not found")?;
    let path = file.file_path.as_deref().filter(|p| !p.is_empty()).ok_or("Photo has no path")?;
    let doc = state(file.edits.as_deref());
    let view;
    let edits = if doc["recipe"].is_object() || doc["auto_enhance"].as_bool().unwrap_or(false) {
        file.edits.as_deref()
    } else {
        view = json!({"recipe": {"sourceFilePath":"","destFilePath":"","outputFormat":"jpg","orientation":1,
            "flipHorizontal":false,"flipVertical":false,"rotate":file.rotate.unwrap_or(0).rem_euclid(360),
            "crop":{"x":0,"y":0,"width":0,"height":0},"resize":{"width":null,"height":null}}}).to_string();
        Some(view.as_str())
    };
    let params = render_params(file_id, path, edits, false)?;
    t_image::get_edited_image(&params).await
}

/// The edits document as it was at `revision`: the current state, or the history or redo entry
/// with that revision (LAP-087). None when that revision is no longer kept.
fn revision_doc(doc: &Value, revision: u64) -> Option<Value> {
    if doc["revision"].as_u64().unwrap_or(0) == revision { return Some(doc.clone()); }
    let entries = doc["history"].as_array().into_iter().chain(doc["redo"].as_array()).flatten();
    entries.filter(|e| e["revision"].as_u64() == Some(revision)).last().map(|e| json!({
        "recipe": e["recipe"], "auto_enhance": e["auto_enhance"], "auto_enhance_preset": e["auto_enhance_preset"],
        "revision": revision,
    }))
}

/// The 2048 px preview with these edits applied (a full edits document as a string).
async fn render_preview_with(file_id: i64, path: &str, orientation: i32, edits: &str, base_only: bool) -> Result<Vec<u8>, String> {
    render_preview_sized(file_id, path, orientation, edits, base_only, None).await
}

async fn render_preview_sized(file_id: i64, path: &str, orientation: i32, edits: &str, base_only: bool, preview_size: Option<u32>) -> Result<Vec<u8>, String> {
    let preview = crate::t_image::get_or_create_preview_image(file_id, path, orientation).await?;
    let (raw_w, raw_h) = t_image::get_image_dimensions(path).unwrap_or((2048, 2048));
    let params = render_params(file_id, path, Some(edits), base_only)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut img = image::load_from_memory(&preview).map_err(|e| format!("Failed to decode preview buffer: {}", e))?;
        if let Some(size) = preview_size { img = img.thumbnail(size.clamp(256, 2048), size.clamp(256, 2048)); }
        let scaled = params.scaled_to_preview(raw_w, raw_h, img.width(), img.height());
        crate::t_jpeg::encode_rgb8(&t_image::apply_edit_params(img, &scaled)?.to_rgb8(), 88)
    }).await.map_err(|e| e.to_string())?
}

/// The preview as it was at edit `revision` (the `er=` a page URL asks for), so a pane showing an
/// earlier state never picks up later edits, e.g. the other side of A|B on the same photo (LAP-087).
/// The current revision, or one no longer kept, renders as `render` does.
pub async fn render_revision(file_id: i64, revision: u64) -> Result<Vec<u8>, String> {
    let file = t_sqlite::AFile::get_file_info(file_id)?.ok_or("Photo not found")?;
    let doc = state(file.edits.as_deref());
    let Some(old) = revision_doc(&doc, revision).filter(|_| doc["revision"].as_u64().unwrap_or(0) != revision) else {
        return render(file_id, false).await;
    };
    let key = format!("edit-rev-v1:{}:{file_id}:{revision}:{}", t_image::ORIENTATION_ALGORITHM_VERSION, file.modified_at.unwrap_or(0));
    if let Some(data) = crate::t_thumb_cache::get_face(&key) { return Ok(data); }
    let _permit = RENDERING.get_or_init(|| tokio::sync::Semaphore::new(2)).acquire().await.map_err(|e| e.to_string())?;
    let path = file.file_path.as_deref().ok_or("Photo has no path")?;
    let data = render_preview_with(file_id, path, file.e_orientation.unwrap_or(1) as i32, &old.to_string(), false).await?;
    crate::t_thumb_cache::put_face(&key, &data)?;
    Ok(data)
}

/// The editor's base preview (as `render_base`) with an unsaved Picasa tool list, for a live
/// preview while an effect's Amount slider moves; nothing is saved and no history step is made.
pub async fn render_draft(file_id: i64, picasa: Value, preview_size: Option<u32>) -> Result<Vec<u8>, String> {
    let file = t_sqlite::AFile::get_file_info(file_id)?.ok_or("Photo not found")?;
    let mut doc = state(file.edits.as_deref());
    if doc["recipe"].is_null() { doc["recipe"] = json!({}); }
    doc["recipe"]["picasa"] = picasa;
    let _permit = RENDERING.get_or_init(|| tokio::sync::Semaphore::new(2)).acquire().await.map_err(|e| e.to_string())?;
    let path = file.file_path.as_deref().ok_or("Photo has no path")?;
    render_preview_sized(file_id, path, file.e_orientation.unwrap_or(1) as i32, &doc.to_string(), true, preview_size).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lap106_fill_light_reaches_sized_draft_thumbnail_and_export() {
        use image::{DynamicImage, GenericImageView, Rgb, RgbImage};
        let root = std::env::temp_dir().join(format!("lap106-fill-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let photo = root.join("photo.png");
        let source = DynamicImage::ImageRgb8(RgbImage::from_pixel(2304,576,Rgb([40,30,20])));
        source.save(&photo).unwrap();
        let path = photo.to_str().unwrap();
        for fill in [0,30,100] {
            let tools = json!({"fillLight":fill});
            let doc = json!({"recipe":{"sourceFilePath":"","destFilePath":"","outputFormat":"png","orientation":1,
                "flipHorizontal":false,"flipVertical":false,"rotate":0,"crop":{"x":0,"y":0,"width":0,"height":0},
                "resize":{"width":null,"height":null},"picasa":tools}}).to_string();
            let expected = crate::t_edit_tools::apply_for(source.clone(), &tools, Some(&photo)).unwrap().to_rgb8().get_pixel(0,0).0;
            if fill == 0 { assert_eq!(expected,[40,30,20]); } else { assert!(expected[0]>40 && expected[1]>30); }
            let params = render_params(0,path,Some(&doc),false).unwrap();
            let export = t_image::get_edited_image(&params).await.unwrap();
            assert_eq!(export.dimensions(),(2304,576));
            assert_eq!(export.to_rgb8().get_pixel(100,100).0,expected);
            let thumbnail = t_image::apply_edit_params(source.thumbnail(512,512),
                &params.scaled_to_preview(2304,576,512,128)).unwrap();
            assert_eq!(thumbnail.to_rgb8().get_pixel(100,50).0,expected);
            for (size,width) in [(Some(0),256),(Some(768),768),(Some(u32::MAX),2048),(None,2048)] {
                let bytes = render_preview_sized(0,path,1,&doc,true,size).await.unwrap();
                let draft = image::load_from_memory(&bytes).unwrap().to_rgb8();
                assert_eq!(draft.dimensions(),(width,width/4));
                let actual = draft.get_pixel(width/2,width/8).0;
                assert!(actual.iter().zip(expected).all(|(a,e)| (*a as i32-e as i32).abs()<=3),"{fill}: {actual:?} vs {expected:?}");
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lap106_automatic_levels_bound_dark_channels_and_preserve_flat_photos() {
        use image::{DynamicImage, Rgb, RgbImage};
        for color in [false, true] {
            for value in [0, 1, 32, 128, 255] {
                let flat = DynamicImage::ImageRgb8(RgbImage::from_pixel(40, 30, Rgb([value;3])));
                assert_eq!(compute_auto_levels(&flat, color), [[0.0,1.0];3]);
            }
            let dark = DynamicImage::ImageRgb8(RgbImage::from_fn(100, 100, |x,y|
                Rgb([(x/2) as u8, (y/8) as u8, 0])));
            let levels=compute_auto_levels(&dark, color);
            for [low,gain] in levels {
                assert!((0.0..=12.0).contains(&low));
                assert!((1.0..=if color {1.08} else {1.25}).contains(&gain));
            }
            let tools=json!({(if color {"autoColor"} else {"autoContrast"}):levels});
            let result=crate::t_edit_tools::apply(dark.clone(), &tools).unwrap().to_rgb8();
            assert!(result.pixels().all(|p| p[2]==0), "a missing channel must stay missing");
            assert_eq!(result,crate::t_edit_tools::apply(dark,&tools).unwrap().to_rgb8());
        }
    }

    #[test]
    fn revision_doc_finds_the_state_a_url_names_and_nothing_newer() {
        // Wand at revision 1, Sepia at 2, Undo back to 1: revision 2 lives on in redo.
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE afiles(id INTEGER PRIMARY KEY,edits TEXT);INSERT INTO afiles VALUES(1,NULL);").unwrap();
        change(&mut conn, 1, EditChange::Enhance(true)).unwrap();
        let sepia = json!({"picasa":{"effects":[{"name":"Sepia","amount":0.5}]}});
        let at2 = state(Some(&change(&mut conn, 1, EditChange::Batch(sepia.clone(), "Sepia")).unwrap()));
        assert_eq!(at2["revision"], 2);
        let doc = state(Some(&change(&mut conn, 1, EditChange::Undo).unwrap()));
        assert_eq!(doc["revision"], 1);
        let one = revision_doc(&doc, 1).unwrap();
        assert_eq!(one["auto_enhance"], true);
        assert!(one["recipe"].is_null());
        let two = revision_doc(&doc, 2).unwrap();
        assert_eq!(two["recipe"], sepia);
        assert_eq!(two["auto_enhance"], true);
        assert_eq!(two["auto_enhance_preset"], 1);
        assert!(revision_doc(&doc, 9).is_none());
        // Before any edit (revision 0, the unedited photo) the history has the empty state.
        let zero = revision_doc(&doc, 0).unwrap();
        assert_eq!(zero["auto_enhance"], false);
    }
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

    #[test]
    fn scaled_to_preview_scales_crop_proportionally() {
        let p = json!({
            "sourceFilePath": "", "destFilePath": "", "outputFormat": "jpg",
            "orientation": 1, "flipHorizontal": false, "flipVertical": false,
            "rotate": 0, "cropAngle": 0.0,
            "crop": {"x": 1200, "y": 800, "width": 2400, "height": 1600},
            "resize": {"width": null, "height": null}
        });
        let params: t_image::EditParams = serde_json::from_value(p).unwrap();
        // Raw: 6000x4000. Preview: 2048x1365 (scale ~0.34133)
        let scaled = params.scaled_to_preview(6000, 4000, 2048, 1365);
        // Expected x: 1200 * 2048 / 6000 = 409.6 -> 410
        // Expected y: 800 * 1365 / 4000 = 273.0 -> 273
        // Expected width: 2400 * 2048 / 6000 = 819.2 -> 819
        // Expected height: 1600 * 1365 / 4000 = 546.0 -> 546
        let val = serde_json::to_value(&scaled).unwrap();
        let c = &val["crop"];
        assert_eq!(c["x"].as_u64().unwrap(), 410);
        assert_eq!(c["y"].as_u64().unwrap(), 273);
        assert_eq!(c["width"].as_u64().unwrap(), 819);
        assert_eq!(c["height"].as_u64().unwrap(), 546);
    }

    #[test]
    fn preview_max_dimension_constant_is_2048() {
        assert_eq!(crate::t_image::PREVIEW_MAX_DIMENSION, 2048);
    }

    #[test]
    fn successive_im_feeling_lucky_clicks_cycle_presets_and_original_and_undo_cleanly() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE afiles(id INTEGER PRIMARY KEY,edits TEXT);INSERT INTO afiles VALUES(1,NULL);").unwrap();
        for expected_preset in 1..=crate::t_enhance::PRESET_COUNT {
            let s = change(&mut conn, 1, EditChange::Enhance(true)).unwrap();
            let doc = state(Some(&s));
            assert_eq!(doc["auto_enhance"], true);
            assert_eq!(doc["auto_enhance_preset"].as_u64().unwrap() as u8, expected_preset);
            assert_eq!(doc["step"], "I'm Feeling Lucky");
        }
        // 6th click cycles back into unedited Original (auto_enhance: false, preset: 0)
        let original = change(&mut conn, 1, EditChange::Enhance(true)).unwrap();
        let orig_doc = state(Some(&original));
        assert_eq!(orig_doc["auto_enhance"], false);
        assert_eq!(orig_doc["auto_enhance_preset"].as_u64().unwrap(), 0);
        // 7th click starts again at Style 1 (Balanced)
        let restarted = change(&mut conn, 1, EditChange::Enhance(true)).unwrap();
        let restart_doc = state(Some(&restarted));
        assert_eq!(restart_doc["auto_enhance"], true);
        assert_eq!(restart_doc["auto_enhance_preset"].as_u64().unwrap(), 1);
        // Undo steps back to Original, and one more Undo steps back to Style 5
        let undo_to_orig = change(&mut conn, 1, EditChange::Undo).unwrap();
        assert_eq!(state(Some(&undo_to_orig))["auto_enhance"], false);
        let undo_to_five = change(&mut conn, 1, EditChange::Undo).unwrap();
        assert_eq!(state(Some(&undo_to_five))["auto_enhance"], true);
        assert_eq!(state(Some(&undo_to_five))["auto_enhance_preset"].as_u64().unwrap(), 5);
    }

    #[test]
    fn lucky_style_buttons_set_one_step_each_and_undo_back() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE afiles(id INTEGER PRIMARY KEY,edits TEXT);INSERT INTO afiles VALUES(1,NULL);").unwrap();
        // Orig on an unedited photo writes nothing
        let same = state(Some(&change(&mut conn, 1, EditChange::Style(0)).unwrap()));
        assert!(same["history"].as_array().is_none_or(|h| h.is_empty()));
        // Orig -> 3 in one click, one history step
        let three = state(Some(&change(&mut conn, 1, EditChange::Style(3)).unwrap()));
        assert_eq!(three["auto_enhance"], true);
        assert_eq!(three["auto_enhance_preset"], 3);
        assert_eq!(three["step"], "I'm Feeling Lucky");
        assert_eq!(three["history"].as_array().unwrap().len(), 1);
        // 3 again is no new step; 3 -> 5 is one more
        let again = state(Some(&change(&mut conn, 1, EditChange::Style(3)).unwrap()));
        assert_eq!(again["history"].as_array().unwrap().len(), 1);
        let five = state(Some(&change(&mut conn, 1, EditChange::Style(5)).unwrap()));
        assert_eq!(five["auto_enhance_preset"], 5);
        // 5 -> Orig, then Undo returns to 5, Undo again to 3, Undo again to the unedited photo
        let orig = state(Some(&change(&mut conn, 1, EditChange::Style(0)).unwrap()));
        assert_eq!(orig["auto_enhance"], false);
        assert_eq!(state(Some(&change(&mut conn, 1, EditChange::Undo).unwrap()))["auto_enhance_preset"], 5);
        assert_eq!(state(Some(&change(&mut conn, 1, EditChange::Undo).unwrap()))["auto_enhance_preset"], 3);
        assert_eq!(state(Some(&change(&mut conn, 1, EditChange::Undo).unwrap()))["auto_enhance"], false);
        // The wand still cycles on from a chosen style
        change(&mut conn, 1, EditChange::Style(4)).unwrap();
        assert_eq!(state(Some(&change(&mut conn, 1, EditChange::Enhance(true)).unwrap()))["auto_enhance_preset"], 5);
    }

    // LAP-044: Paste All Effects / Batch Edit history semantics.
    fn edits_conn(doc:&Value)->rusqlite::Connection {
        let conn=rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE afiles(id INTEGER PRIMARY KEY,edits TEXT);").unwrap();
        conn.execute("INSERT INTO afiles VALUES(1,?1)",[doc.to_string()]).unwrap();
        conn
    }
    fn lap044_target()->Value {
        json!({"sourceFilePath":"","destFilePath":"","outputFormat":"jpg","orientation":1,"rotate":90,
            "flipHorizontal":true,"flipVertical":false,"cropAngle":0.0,"crop":{"x":5,"y":4,"width":50,"height":70},
            "resize":{"width":null,"height":null},"redEye":true,
            "picasa":{"retouch":[{"sx":0.2,"sy":0.5,"tx":0.8,"ty":0.5}],"effects":[{"name":"Tint","amount":0.3}],"lastStep":"Tint"}})
    }

    #[test]
    fn paste_is_one_undo_step_with_auto_enhance_and_undo_restores_exact_prior_state() {
        let target=lap044_target();
        let mut conn=edits_conn(&json!({"recipe":target,"auto_enhance":false,"revision":4,"next_revision":4,"step":"Tint","history_version":2,"history":[],"redo":[]}));
        let mut pasted=target.clone();
        pasted["cropAngle"]=json!(3.0);
        pasted["picasa"]["effects"].as_array_mut().unwrap().push(json!({"name":"Sepia","amount":0.5}));
        pasted["picasa"]["lastStep"]=json!("Paste All Effects");
        validate_geometry(&pasted,80,60).unwrap();
        let after=state(Some(&change(&mut conn,1,EditChange::Paste(pasted.clone(),Some(3))).unwrap()));
        assert_eq!(after["recipe"],pasted);
        assert_eq!(after["auto_enhance"],true);
        assert_eq!(after["auto_enhance_preset"],3,"the copied photo's I'm Feeling Lucky style comes with the paste");
        assert_eq!(after["step"],"Paste All Effects");
        let history=after["history"].as_array().unwrap();
        assert_eq!(history.len(),1,"paste + autoEnhance must be a single undo entry");
        assert_eq!(history[0]["recipe"],target);
        assert_eq!(history[0]["auto_enhance"],false);
        let undo=state(Some(&change(&mut conn,1,EditChange::Undo).unwrap()));
        assert_eq!(undo["recipe"],target);
        assert_eq!(undo["auto_enhance"],false);
        assert_eq!(undo["revision"],4);
        assert_eq!(undo["step"],"Tint");
        assert!(undo["history"].as_array().unwrap().is_empty());
        let redo=state(Some(&change(&mut conn,1,EditChange::Redo).unwrap()));
        assert_eq!(redo["recipe"],pasted);
        assert_eq!(redo["auto_enhance"],true);
        assert_eq!(redo["auto_enhance_preset"],3);
    }

    #[test]
    fn copy_carries_the_lucky_style_and_batch_lucky_is_always_style_1() {
        let effects=effects_from_doc(&json!({"recipe":lap044_target(),"auto_enhance":true,"auto_enhance_preset":4}));
        assert_eq!(effects["autoEnhance"],true);
        assert_eq!(effects["autoEnhancePreset"],4);
        assert_eq!(effects_from_doc(&json!({"recipe":lap044_target()}))["autoEnhancePreset"],0);
        // Batch Edit > I'm Feeling Lucky on a photo already at Style 4: Style 1, not the editor's next style.
        let mut conn=edits_conn(&json!({"recipe":lap044_target(),"auto_enhance":true,"auto_enhance_preset":4,"history_version":2}));
        let after=state(Some(&change(&mut conn,1,EditChange::Lucky).unwrap()));
        assert_eq!(after["auto_enhance"],true);
        assert_eq!(after["auto_enhance_preset"],1);
        assert_eq!(after["step"],"I'm Feeling Lucky");
        let undo=state(Some(&change(&mut conn,1,EditChange::Undo).unwrap()));
        assert_eq!(undo["auto_enhance_preset"],4);
    }

    #[test]
    fn paste_without_auto_enhance_keeps_targets_auto_enhance() {
        let target=lap044_target();
        let mut conn=edits_conn(&json!({"recipe":target,"auto_enhance":true,"history_version":2}));
        let mut pasted=target.clone();
        pasted["brightness"]=json!(12);
        let after=state(Some(&change(&mut conn,1,EditChange::Paste(pasted,None)).unwrap()));
        assert_eq!(after["auto_enhance"],true);
        assert_eq!(after["history"].as_array().unwrap().len(),1);
    }

    #[test]
    fn batch_tools_are_one_labelled_undo_step_each_and_undo_all_is_undoable() {
        let target=lap044_target();
        let mut conn=edits_conn(&json!({"recipe":target,"auto_enhance":true,"history_version":2}));
        let mut sepia=target.clone();
        sepia["picasa"]["effects"].as_array_mut().unwrap().push(json!({"name":"Sepia","amount":0.5}));
        sepia["picasa"]["lastStep"]=json!("Sepia");
        let first=state(Some(&change(&mut conn,1,EditChange::Batch(sepia.clone(),"Sepia")).unwrap()));
        assert_eq!(first["step"],"Sepia");
        assert_eq!(first["history"].as_array().unwrap().len(),1);
        let mut bw=sepia.clone();
        bw["picasa"]["effects"].as_array_mut().unwrap().push(json!({"name":"B&W","amount":0.5}));
        bw["picasa"]["lastStep"]=json!("B&W");
        let second=state(Some(&change(&mut conn,1,EditChange::Batch(bw.clone(),"B&W")).unwrap()));
        assert_eq!(second["step"],"B&W");
        assert_eq!(second["history"].as_array().unwrap().len(),2);
        let undo=state(Some(&change(&mut conn,1,EditChange::Undo).unwrap()));
        assert_eq!(undo["recipe"],sepia);
        let reset=state(Some(&change(&mut conn,1,EditChange::Reset).unwrap()));
        assert!(reset["recipe"].is_null());
        assert_eq!(reset["auto_enhance"],false);
        assert_eq!(reset["step"],"Undo All Edits");
        let undo_reset=state(Some(&change(&mut conn,1,EditChange::Undo).unwrap()));
        assert_eq!(undo_reset["recipe"],sepia);
        assert_eq!(undo_reset["auto_enhance"],true);
    }

    #[test]
    fn pasted_straighten_can_push_existing_crop_out_of_bounds() {
        // 80x60 at 10 degrees -> 90x73 canvas; at 2 degrees -> 83x63.
        let mut p=json!({"orientation":1,"rotate":0,"cropAngle":10.0,"crop":{"x":0,"y":0,"width":88,"height":72}});
        validate_geometry(&p,80,60).unwrap();
        p["cropAngle"]=json!(2.0);
        assert_eq!(validate_geometry(&p,80,60),Err("Crop is outside the image".to_string()));
    }

    #[test]
    fn copy_keeps_straighten_tone_and_effects_but_not_crop_rotate_redeye_retouch() {
        let doc=json!({"auto_enhance":true,"recipe":{"orientation":1,"rotate":90,"flipHorizontal":true,"flipVertical":true,
            "cropAngle":4.5,"crop":{"x":5,"y":4,"width":50,"height":70},"resize":{"width":100,"height":140},"redEye":true,
            "filter":"sepia","brightness":10,"contrast":15.0,"blur":1.5,"hue_rotate":20,"saturation":1.2,
            "picasa":{"retouch":[{"sx":0.2}],"effects":[{"name":"Tint","amount":0.3}],"text":[{"png":"t"}],"fillLight":20,
                "highlights":5,"shadows":-5,"temperature":7,"neutral":[1.0,0.9,1.1],"curves":{"rgb":[[0,0],[255,255]]},"lastStep":"Tint"}}});
        let e=effects_from_doc(&doc);
        let mut keys:Vec<_>=e.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys,["autoEnhance","autoEnhancePreset","blur","brightness","contrast","cropAngle","filter","hue_rotate","picasa","saturation"]);
        assert_eq!(e["cropAngle"],4.5);
        assert_eq!(e["autoEnhance"],true);
        assert_eq!(e["autoEnhancePreset"],1,"Lucky on with no preset stored (before LAP-085) copies as Style 1");
        for k in ["filter","brightness","contrast","blur","hue_rotate","saturation"] {assert_eq!(e[k],doc["recipe"][k],"{k}");}
        let mut tools=doc["recipe"]["picasa"].clone();
        tools.as_object_mut().unwrap().remove("retouch");
        tools.as_object_mut().unwrap().remove("lastStep");
        assert_eq!(e["picasa"],tools);
        // No recipe: defaults only.
        assert_eq!(effects_from_doc(&json!({})),json!({"cropAngle":0.0,"filter":null,"brightness":null,"contrast":null,
            "blur":null,"hue_rotate":null,"saturation":null,"autoEnhance":false,"autoEnhancePreset":0,"picasa":null}));
    }

    #[test]
    fn merge_adds_non_default_effects_and_keeps_target_geometry_and_retouch() {
        let mut base=lap044_target();
        base["brightness"]=json!(5);base["contrast"]=json!(30.0);base["resize"]=json!({"width":100,"height":140});
        base["picasa"]["fillLight"]=json!(10);base["picasa"]["text"]=json!([{"png":"t0"}]);
        // Copied effects, plus geometry keys a caller might smuggle in: they must be ignored.
        let effects=json!({"cropAngle":3.0,"filter":"","brightness":0,"contrast":12.0,"blur":0.0,"hue_rotate":0,"saturation":1.0,
            "autoEnhance":false,"crop":{"x":0,"y":0,"width":1,"height":1},"rotate":180,"redEye":false,"flipHorizontal":false,
            "picasa":{"fillLight":0,"shadows":25,"effects":[{"name":"Sepia","amount":0.5}],"text":[{"png":"t1"}],"retouch":[]}});
        let out=merge_effects(Some(&base),6,&effects);
        for k in ["crop","rotate","flipHorizontal","flipVertical","redEye","resize","brightness"] {assert_eq!(out[k],base[k],"{k} must keep target");}
        assert_eq!(out["picasa"]["retouch"],base["picasa"]["retouch"]);
        assert_eq!(out["cropAngle"],3.0);
        assert_eq!(out["contrast"],12.0);
        assert!(out.get("filter").is_none() && out.get("blur").is_none() && out.get("saturation").is_none() && out.get("hue_rotate").is_none());
        assert_eq!(out["picasa"]["fillLight"],10,"default copied fillLight keeps target's");
        assert_eq!(out["picasa"]["shadows"],25.0);
        assert_eq!(out["picasa"]["effects"],json!([{"name":"Tint","amount":0.3},{"name":"Sepia","amount":0.5}]));
        assert_eq!(out["picasa"]["text"],json!([{"png":"t0"},{"png":"t1"}]));
        assert_eq!(out["picasa"]["lastStep"],"Paste All Effects");
        assert_eq!(out["orientation"],6);
        assert_eq!(out["sourceFilePath"],"");
        // Unedited target: default geometry plus the pasted effects.
        let fresh=merge_effects(None,1,&effects);
        assert_eq!(fresh["crop"],json!({"x":0,"y":0,"width":0,"height":0}));
        assert_eq!(fresh["rotate"],0);
        assert_eq!(fresh["redEye"],false);
        assert_eq!(fresh["cropAngle"],3.0);
        assert_eq!(fresh["picasa"]["effects"],json!([{"name":"Sepia","amount":0.5}]));
    }

    #[test]
    fn paste_reports_out_of_bounds_and_missing_photos_failed_and_still_applies_others() {
        let doc1=json!({"recipe":{"orientation":1,"rotate":0,"cropAngle":10.0,"crop":{"x":0,"y":0,"width":88,"height":72},
            "resize":{"width":null,"height":null},"picasa":{}},"history_version":2}).to_string();
        let doc2=json!({"recipe":lap044_target(),"history_version":2}).to_string();
        let mut conn=rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE afiles(id INTEGER PRIMARY KEY,edits TEXT);").unwrap();
        conn.execute("INSERT INTO afiles VALUES(1,?1),(2,?2)",[&doc1,&doc2]).unwrap();
        let effects=json!({"cropAngle":2.0,"autoEnhance":true,"picasa":{"effects":[{"name":"Sepia","amount":0.5}]}});
        let files=[(1,Some(doc1.clone()),80,60,1),(2,Some(doc2.clone()),80,60,1),(3,None,80,60,1)];
        let res=paste_effects_on(&mut conn,&files,&effects);
        assert_eq!(res["ok"],json!([2]));
        let failed=res["failed"].as_array().unwrap();
        assert_eq!(failed.len(),2);
        assert_eq!(failed[0]["id"],1);
        assert_eq!(failed[0]["error"],"Crop is outside the image");
        assert_eq!(failed[1]["id"],3);
        let read=|id:i64|->String {conn.query_row("SELECT edits FROM afiles WHERE id=?1",[id],|r|r.get(0)).unwrap()};
        assert_eq!(read(1),doc1,"failed photo must be left untouched");
        let after=state(Some(&read(2)));
        assert_eq!(after["recipe"]["crop"],lap044_target()["crop"]);
        assert_eq!(after["recipe"]["rotate"],90);
        assert_eq!(after["recipe"]["redEye"],true);
        assert_eq!(after["recipe"]["cropAngle"],2.0);
        assert_eq!(after["recipe"]["picasa"]["effects"],json!([{"name":"Tint","amount":0.3},{"name":"Sepia","amount":0.5}]));
        assert_eq!(after["auto_enhance"],true);
        assert_eq!(after["step"],"Paste All Effects");
        assert_eq!(after["history"].as_array().unwrap().len(),1);
        let undo=state(Some(&change(&mut conn,2,EditChange::Undo).unwrap()));
        assert_eq!(undo["recipe"],lap044_target());
        assert_eq!(undo["auto_enhance"],false);
    }

    #[test]
    fn tool_recipe_appends_named_effect_and_rejects_non_tools() {
        let (r,label)=tool_recipe(init_recipe(None,1),"sepia").unwrap();
        assert_eq!(label,"Sepia");
        assert_eq!(r["picasa"]["effects"],json!([{"name":"Sepia","amount":0.5}]));
        assert_eq!(r["picasa"]["lastStep"],"Sepia");
        let (r,label)=tool_recipe(lap044_target(),"bw").unwrap();
        assert_eq!(label,"B&W");
        assert_eq!(r["picasa"]["effects"],json!([{"name":"Tint","amount":0.3},{"name":"B&W","amount":0.5}]));
        assert_eq!(r["crop"],lap044_target()["crop"]);
        for (op,name) in [("sharpen","Sharpen"),("warmify","Warmify"),("film_grain","Film Grain")] {
            assert_eq!(tool_recipe(json!({}),op).unwrap().1,name);
        }
        for op in ["lucky","undo_all","auto_contrast","Sepia","","bogus"] {assert!(tool_recipe(json!({}),op).is_err(),"{op}");}
    }

    #[test]
    fn editor_first_lucky_click_and_batch_lucky_render_the_same_pixels() {
        // LAP-043: the editor's first I'm Feeling Lucky click and Batch Edit > I'm Feeling Lucky
        // must leave the same state (iAuto 2019, Style 1), so every render path draws the same
        let mut editor = rusqlite::Connection::open_in_memory().unwrap();
        editor.execute_batch("CREATE TABLE afiles(id INTEGER PRIMARY KEY,edits TEXT);INSERT INTO afiles VALUES(1,NULL);").unwrap();
        let mut batch = rusqlite::Connection::open_in_memory().unwrap();
        batch.execute_batch("CREATE TABLE afiles(id INTEGER PRIMARY KEY,edits TEXT);INSERT INTO afiles VALUES(1,NULL);").unwrap();
        let a = change(&mut editor, 1, EditChange::Enhance(true)).unwrap();
        let b = change(&mut batch, 1, EditChange::Lucky).unwrap();
        let (sa, sb) = (state(Some(&a)), state(Some(&b)));
        for key in ["auto_enhance", "auto_enhance_preset", "recipe", "step", "revision"] { assert_eq!(sa[key], sb[key], "{key}"); }
        assert_eq!(crate::t_enhance::extract_auto_enhance_preset(&a), 1);
        let photo = image::RgbImage::from_fn(64, 48, |x, y| image::Rgb([(40 + x * 2) as u8, (30 + y * 3) as u8, (90 + x + y) as u8]));
        let render = |edits: &str| {
            let mut img = image::DynamicImage::ImageRgb8(photo.clone());
            crate::t_enhance::apply_auto_enhance_preset(&mut img, crate::t_enhance::extract_auto_enhance_preset(edits));
            img.into_rgb8().into_raw()
        };
        assert_eq!(render(&a), render(&b));
    }

    #[test]
    fn batch_edit_rejects_unknown_op_before_opening_the_library() {
        // The exact message proves it returned before open_conn()/get_file_info().
        assert_eq!(batch_edit(&[1,2],"bogus"),Err("Unknown batch edit operation: bogus".to_string()));
        assert_eq!(batch_edit(&[],"SEPIA"),Err("Unknown batch edit operation: SEPIA".to_string()));
    }

    #[test]
    fn automask_reaches_the_base_render_and_only_whole_photo_entries_copy() {
        let doc=json!({"recipe":{"picasa":{"local":[{"region":"sky","mask":"0123456789abcdef0123456789abcdef","highlights":-50},
            {"region":"whole","mask":null,"shadows":30}]}}});
        assert!(has_base_recipe_tools(&doc), "the editor's base preview must render local curves");
        assert!(!has_base_recipe_tools(&json!({"recipe":{"picasa":{"local":[]}}})));
        assert_eq!(effects_from_doc(&doc)["picasa"]["local"],json!([{"region":"whole","mask":null,"shadows":30}]));
    }

    #[test]
    fn public_041_tools_survive_current_history_reads_and_undo() {
        let legacy=json!({"recipe":{"tools":{"fillLight":27}},"history_version":2,
            "history":[{"recipe":{"tools":{"fillLight":8}},"revision":2,"step":"Fill Light"}],
            "redo":[{"recipe":{"tools":{"fillLight":40}},"revision":4}],"revision":3,
            "unknown":{"tools":{"keep":"unrelated"}}}).to_string();
        let doc=state(Some(&legacy));
        assert_eq!(doc["recipe"]["picasa"]["fillLight"],27);
        assert_eq!(doc["history"][0]["recipe"]["picasa"]["fillLight"],8);
        assert_eq!(doc["redo"][0]["recipe"]["picasa"]["fillLight"],40);
        assert_eq!(doc["unknown"]["tools"]["keep"],"unrelated");
        assert!(doc["recipe"].get("tools").is_none());
        assert!(serde_json::from_str::<Value>(&legacy).unwrap()["recipe"].get("tools").is_some());
        assert_eq!(state(Some(&json!({"history":[{"tools":{"fillLight":8}}]}).to_string()))["history"][0]["picasa"]["fillLight"],8);
        let mut conn=rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE afiles(id INTEGER PRIMARY KEY,edits TEXT)").unwrap();
        conn.execute("INSERT INTO afiles VALUES(1,?1)",[legacy]).unwrap();
        let undone=state(Some(&change(&mut conn,1,EditChange::Undo).unwrap()));
        assert_eq!(undone["recipe"]["picasa"]["fillLight"],8);
        let redone=state(Some(&change(&mut conn,1,EditChange::Redo).unwrap()));
        assert_eq!(redone["recipe"]["picasa"]["fillLight"],27);
    }

    #[test]
    fn current_tool_key_wins_and_legacy_export_params_remain_readable() {
        let current=json!({"recipe":{"picasa":{"fillLight":7},"tools":{"fillLight":99}}});
        let doc=state(Some(&current.to_string()));
        assert_eq!(doc["recipe"]["picasa"]["fillLight"],7);
        assert!(doc["recipe"].get("tools").is_none());
        let params=json!({"sourceFilePath":"","destFilePath":"","outputFormat":"png","orientation":1,
            "flipHorizontal":false,"flipVertical":false,"rotate":0,
            "crop":{"x":0,"y":0,"width":0,"height":0},"resize":{"width":null,"height":null},
            "tools":{"fillLight":27}});
        let decoded:t_image::EditParams=serde_json::from_value(params).unwrap();
        let saved=serde_json::to_value(decoded).unwrap();
        assert_eq!(saved["picasa"]["fillLight"],27);
        assert!(saved.get("tools").is_none());
    }
}
