//! Bounded, shape-preserving point curves shared by every recipe render path.
pub fn lut(points: &[[f64; 2]]) -> Result<[u8; 256], String> {
    if !(2..=16).contains(&points.len()) || points.first() != Some(&[0.0, 0.0])
        || points.last() != Some(&[1.0, 1.0])
        || points.iter().flatten().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        || points.windows(2).any(|p| p[1][0] - p[0][0] < 1.0 / 255.0)
    { return Err("Invalid curve points".into()); }
    let slopes: Vec<_> = points.windows(2).map(|p| (p[1][1]-p[0][1])/(p[1][0]-p[0][0])).collect();
    let mut tangents = vec![0.0; points.len()];
    tangents[0] = slopes[0];
    tangents[points.len()-1] = *slopes.last().unwrap();
    for i in 1..points.len()-1 {
        if slopes[i-1]*slopes[i] > 0.0 {
            let left=points[i][0]-points[i-1][0];
            let right=points[i+1][0]-points[i][0];
            let a=2.0*right+left; let b=right+2.0*left;
            tangents[i]=(a+b)/(a/slopes[i-1]+b/slopes[i]);
        }
    }
    for i in 0..slopes.len() {
        if slopes[i] == 0.0 { tangents[i]=0.0; tangents[i+1]=0.0; continue; }
        let norm=(tangents[i]/slopes[i]).powi(2)+(tangents[i+1]/slopes[i]).powi(2);
        if norm > 9.0 {
            let factor=3.0/norm.sqrt(); tangents[i]*=factor; tangents[i+1]*=factor;
        }
    }
    Ok(std::array::from_fn(|v| {
        let x=v as f64/255.0;
        let i=points.windows(2).position(|p| x<=p[1][0]).unwrap_or(points.len()-2);
        let h=points[i+1][0]-points[i][0]; let t=(x-points[i][0])/h;
        let y=(2.0*t.powi(3)-3.0*t*t+1.0)*points[i][1]
            +(t.powi(3)-2.0*t*t+t)*h*tangents[i]
            +(-2.0*t.powi(3)+3.0*t*t)*points[i+1][1]
            +(t.powi(3)-t*t)*h*tangents[i+1];
        (y.clamp(0.0,1.0)*255.0).round() as u8
    }))
}

pub fn map(pixel: [u8; 3], tables: &[[u8; 256]; 4]) -> [u8; 3] {
    std::array::from_fn(|c| tables[c+1][tables[0][pixel[c] as usize] as usize])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_and_master_then_channel_isolation() {
        let identity=lut(&[[0.0,0.0],[1.0,1.0]]).unwrap();
        assert_eq!(identity,std::array::from_fn(|i|i as u8));
        let red=lut(&[[0.0,0.0],[0.5,0.75],[1.0,1.0]]).unwrap();
        let tables=[identity,red,identity,identity];
        assert_eq!(map([128,90,70],&tables),[red[128],90,70]);
        let master=lut(&[[0.0,0.0],[0.5,0.25],[1.0,1.0]]).unwrap();
        assert_eq!(map([128;3],&[master,red,identity,identity]),[red[master[128] as usize],master[128],master[128]]);
    }
    #[test]
    fn smooth_monotonic_points_stay_bounded_and_flat_spans_stay_flat() {
        let table=lut(&[[0.0,0.0],[0.2,0.1],[0.4,0.1],[0.8,0.95],[1.0,1.0]]).unwrap();
        assert!(table.windows(2).all(|p|p[1]>=p[0]));
        assert!(table[51..=102].iter().all(|v|*v==26));
        assert_eq!((table[0],table[255]),(0,255));
        assert!(lut(&[[0.0,0.0],[0.0,0.4],[1.0,1.0]]).is_err());
        assert!(lut(&[[0.0,0.0],[0.5,f64::NAN],[1.0,1.0]]).is_err());
    }
}
