// lap-render::geometry
// Closed-form trigonometric straighten and auto-crop calculation.
// Analyzed from Google Picasa's CropFilter (0x0041ca85_CropFilter_parse.c).

use serde::{Deserialize, Serialize};

/// Auto-inscribed crop rectangle in the rotated image's axis-aligned bounding box (AABB) coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct InscribedCrop {
    /// Top-left X coordinate within the rotated AABB
    pub x: f64,
    /// Top-left Y coordinate within the rotated AABB
    pub y: f64,
    /// Width of the inscribed crop rectangle
    pub width: f64,
    /// Height of the inscribed crop rectangle
    pub height: f64,
}

/// Computes the maximum axis-aligned rectangle inscribed within a rotated rectangle (W x H)
/// rotated by `angle_deg`, strictly preserving the original image's aspect ratio (W / H).
///
/// Guaranteed properties:
/// 1. Exactly O(1) closed-form calculation with zero latency.
/// 2. Exact preservation of original aspect ratio (W / H).
/// 3. Zero black borders or transparent corner triangles exposed.
pub fn calculate_inscribed_crop(width: f64, height: f64, angle_deg: f64) -> InscribedCrop {
    calculate_inscribed_crop_with_aspect(width, height, angle_deg, None)
}

/// Computes the maximum inscribed crop rectangle with an optional aspect ratio constraint.
/// If `aspect_ratio` is None or <= 0.0, the photo's exact original aspect ratio (width / height) is preserved.
pub fn calculate_inscribed_crop_with_aspect(
    width: f64,
    height: f64,
    angle_deg: f64,
    aspect_ratio: Option<f64>,
) -> InscribedCrop {
    if width <= 0.0 || height <= 0.0 {
        return InscribedCrop {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        };
    }

    let theta = angle_deg.abs().to_radians();
    let r = match aspect_ratio {
        Some(ar) if ar > 0.0 => ar,
        _ => width / height,
    };

    if theta < 1e-9 {
        let (w, h) = if let Some(ar) = aspect_ratio.filter(|&a| a > 0.0) {
            let mut w = width;
            let mut h = w / ar;
            if h > height {
                h = height;
                w = h * ar;
            }
            (w, h)
        } else {
            (width, height)
        };
        return InscribedCrop {
            x: (width - w) / 2.0,
            y: (height - h) / 2.0,
            width: w,
            height: h,
        };
    }

    let cos_t = theta.cos();
    let sin_t = theta.sin();

    // Rotated image axis-aligned bounding box (AABB) dimensions
    let bb_w = width * cos_t + height * sin_t;
    let bb_h = height * cos_t + width * sin_t;

    // Closed-form trigonometric inscribed dimensions:
    // Constraint 1 (fits inside width):  w * cos + h * sin <= width  => w <= width / (cos + sin / r)
    // Constraint 2 (fits inside height): w * sin + h * cos <= height => w <= height / (sin + cos / r)
    let w_by_right = width / (cos_t + sin_t / r);
    let w_by_bottom = height / (sin_t + cos_t / r);
    let w = w_by_right.min(w_by_bottom);
    let h = w / r;

    let x = (bb_w - w) / 2.0;
    let y = (bb_h - h) / 2.0;

    InscribedCrop {
        x,
        y,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_angle_returns_full_image() {
        let crop = calculate_inscribed_crop(1920.0, 1080.0, 0.0);
        assert!((crop.x - 0.0).abs() < 1e-9);
        assert!((crop.y - 0.0).abs() < 1e-9);
        assert!((crop.width - 1920.0).abs() < 1e-9);
        assert!((crop.height - 1080.0).abs() < 1e-9);
    }

    #[test]
    fn test_aspect_ratio_preservation_landscape_0_to_45_deg() {
        let dimensions = [(1920.0, 1080.0), (4000.0, 3000.0), (1600.0, 900.0)];
        for (w, h) in dimensions {
            let original_ratio = w / h;
            let mut angle = 0.0;
            while angle <= 45.0 + 1e-6 {
                let crop = calculate_inscribed_crop(w, h, angle);
                let crop_ratio = crop.width / crop.height;
                assert!(
                    (crop_ratio - original_ratio).abs() < 1e-10,
                    "Landscape ({w}x{h}) at angle {angle}deg failed aspect ratio: expected {original_ratio}, got {crop_ratio}"
                );
                angle += 0.5;
            }
        }
    }

    #[test]
    fn test_aspect_ratio_preservation_portrait_0_to_45_deg() {
        let dimensions = [(1080.0, 1920.0), (3000.0, 4000.0), (900.0, 1600.0)];
        for (w, h) in dimensions {
            let original_ratio = w / h;
            let mut angle = 0.0;
            while angle <= 45.0 + 1e-6 {
                let crop = calculate_inscribed_crop(w, h, angle);
                let crop_ratio = crop.width / crop.height;
                assert!(
                    (crop_ratio - original_ratio).abs() < 1e-10,
                    "Portrait ({w}x{h}) at angle {angle}deg failed aspect ratio: expected {original_ratio}, got {crop_ratio}"
                );
                angle += 0.5;
            }
        }
    }

    #[test]
    fn test_aspect_ratio_preservation_square_0_to_45_deg() {
        let (w, h) = (1000.0, 1000.0);
        let original_ratio = 1.0;
        let mut angle = 0.0;
        while angle <= 45.0 + 1e-6 {
            let crop = calculate_inscribed_crop(w, h, angle);
            let crop_ratio = crop.width / crop.height;
            assert!(
                (crop_ratio - original_ratio).abs() < 1e-10,
                "Square ({w}x{h}) at angle {angle}deg failed aspect ratio: expected {original_ratio}, got {crop_ratio}"
            );
            angle += 0.5;
        }
    }

    #[test]
    fn test_negative_angles_symmetry() {
        let (w, h) = (1920.0, 1080.0);
        for angle in [1.0, 5.0, 15.0, 25.5, 45.0] {
            let pos = calculate_inscribed_crop(w, h, angle);
            let neg = calculate_inscribed_crop(w, h, -angle);
            assert!((pos.x - neg.x).abs() < 1e-9);
            assert!((pos.y - neg.y).abs() < 1e-9);
            assert!((pos.width - neg.width).abs() < 1e-9);
            assert!((pos.height - neg.height).abs() < 1e-9);
        }
    }

    #[test]
    fn test_no_black_borders_across_all_angles() {
        // Geometric containment: when the inscribed rectangle is rotated back into
        // the unrotated image frame, its bounding extents must not exceed [-W/2, W/2] x [-H/2, H/2].
        for (w, h) in [(1920.0, 1080.0), (1080.0, 1920.0), (2000.0, 2000.0)] {
            let mut angle = 0.0;
            while angle <= 45.0 + 1e-6 {
                let crop = calculate_inscribed_crop(w, h, angle);
                let theta = angle.to_radians();
                let cos_t = theta.cos();
                let sin_t = theta.sin();

                // Projections of inscribed rect half-extents onto the original coordinate axes:
                let x_proj = crop.width * cos_t + crop.height * sin_t;
                let y_proj = crop.width * sin_t + crop.height * cos_t;

                assert!(
                    x_proj <= w + 1e-9,
                    "Black border on width at angle {angle}deg: x_proj {x_proj} > w {w}"
                );
                assert!(
                    y_proj <= h + 1e-9,
                    "Black border on height at angle {angle}deg: y_proj {y_proj} > h {h}"
                );

                // AABB containment
                let bb_w = w * cos_t + h * sin_t;
                let bb_h = h * cos_t + w * sin_t;
                assert!(crop.x >= 0.0, "Negative crop.x at angle {angle}deg");
                assert!(crop.y >= 0.0, "Negative crop.y at angle {angle}deg");
                assert!(
                    crop.x + crop.width <= bb_w + 1e-9,
                    "Crop extends past bb_w at angle {angle}deg"
                );
                assert!(
                    crop.y + crop.height <= bb_h + 1e-9,
                    "Crop extends past bb_h at angle {angle}deg"
                );

                angle += 1.0;
            }
        }
    }

    #[test]
    fn test_monotonic_dimension_decrease_with_angle() {
        let (w, h) = (1920.0, 1080.0);
        let mut prev_w = w;
        let mut prev_h = h;
        for angle in [1.0, 5.0, 10.0, 20.0, 30.0, 40.0, 45.0] {
            let crop = calculate_inscribed_crop(w, h, angle);
            assert!(
                crop.width < prev_w,
                "Width should strictly decrease: {prev_w} -> {}",
                crop.width
            );
            assert!(
                crop.height < prev_h,
                "Height should strictly decrease: {prev_h} -> {}",
                crop.height
            );
            prev_w = crop.width;
            prev_h = crop.height;
        }
    }

    #[test]
    fn test_custom_aspect_ratio_preservation() {
        let (w, h) = (1920.0, 1080.0);
        for target_aspect in [1.0, 4.0 / 3.0, 16.0 / 9.0, 9.0 / 16.0] {
            for angle in [0.0, 5.0, 15.0, 30.0, 45.0] {
                let crop = calculate_inscribed_crop_with_aspect(w, h, angle, Some(target_aspect));
                let ratio = crop.width / crop.height;
                assert!(
                    (ratio - target_aspect).abs() < 1e-10,
                    "Failed custom aspect ratio {target_aspect} at angle {angle}deg: got {ratio}"
                );
            }
        }
    }
}
