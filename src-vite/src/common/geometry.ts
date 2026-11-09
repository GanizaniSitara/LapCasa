// Geometry helpers for the image editor.

export interface InscribedRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * Compute the largest axis-aligned rectangle that fits inside a `width`×`height`
 * rectangle rotated by `angleDeg` (in degrees, |angle| ≤ 45°).
 *
 * The result is expressed in the rotated image's axis-aligned bounding box space
 * (i.e. the coordinate space the backend produces after rotating the image), and
 * is centered inside that bounding box. `x`/`y` are the top-left corner of the
 * inscribed rectangle within the bounding box.
 *
 * When `aspectRatio` (width/height) is provided, the rectangle is constrained to
 * that ratio; otherwise the free-aspect maximum-area rectangle is returned.
 *
 * For angle 0 this returns the full image (or the largest aspect-ratio-constrained
 * rectangle for a fixed ratio).
 */
/**
 * Exact closed-form trigonometric inscribed crop calculation matching Google Picasa
 * and lap-render::geometry (0x0041ca85_CropFilter_parse.c).
 *
 * Given image dimensions (W, H) and rotation angle theta:
 *   w_inscribed = (W * H) / (W * sin|theta| + H * cos|theta|)
 *   h_inscribed = w_inscribed * (H / W)
 *
 * Guarantees:
 * 1. Exactly O(1) zero-latency calculation on every mouse-move event.
 * 2. Preserves the photo's exact original aspect ratio (W / H).
 * 3. No black borders or transparent corners are ever exposed.
 */
export function calculateInscribedCrop(
  width: number,
  height: number,
  angleDeg: number,
  aspectRatio?: number,
): InscribedCrop {
  if (width <= 0 || height <= 0) {
    return { x: 0, y: 0, width: 0, height: 0 };
  }

  const theta = (Math.abs(angleDeg) * Math.PI) / 180;
  const hasAspect = aspectRatio != null && aspectRatio > 0;
  const R = hasAspect ? aspectRatio : width / height;

  if (theta < 1e-9) {
    let w = width;
    let h = w / R;
    if (h > height) {
      h = height;
      w = h * R;
    }
    return {
      x: (width - w) / 2,
      y: (height - h) / 2,
      width: w,
      height: h,
    };
  }

  const W = width;
  const H = height;
  const c = Math.cos(theta);
  const s = Math.sin(theta);

  // Bounding box of the rotated image (the space the backend crops from).
  const bbW = W * c + H * s;
  const bbH = H * c + W * s;

  // Closed-form trigonometric inscribed rectangle with aspect ratio R:
  // Constraint 1 (fits inside width):  w * c + h * s <= W  => w <= W / (c + s / R)
  // Constraint 2 (fits inside height): w * s + h * c <= H  => w <= H / (s + c / R)
  const xByRight = W / (2 * (c + s / R));
  const xByBottom = H / (2 * (s + c / R));
  const xHalf = Math.min(xByRight, xByBottom);
  const w = 2 * xHalf;
  const h = w / R;

  return {
    x: (bbW - w) / 2,
    y: (bbH - h) / 2,
    width: w,
    height: h,
  };
}

/**
 * Compute the largest axis-aligned rectangle that fits inside a `width`×`height`
 * rectangle rotated by `angleDeg` (in degrees, |angle| ≤ 45°).
 *
 * Uses the closed-form trigonometric formula (O(1)) ensuring zero latency at 120 FPS
 * and exact preservation of the photo's aspect ratio.
 */
export function getMaxInscribedRect(
  width: number,
  height: number,
  angleDeg: number,
  aspectRatio?: number,
): InscribedRect {
  const rect = calculateInscribedCrop(width, height, angleDeg, aspectRatio);
  if (rect.width <= 0 || rect.height <= 0) return rect;

  const theta = (Math.abs(angleDeg) * Math.PI) / 180;
  if (theta < 1e-9) return rect;

  const W = width;
  const H = height;
  const c = Math.cos(theta);
  const s = Math.sin(theta);
  const bbW = W * c + H * s;
  const bbH = H * c + W * s;

  // Shrink by a ~1px safety margin so the crop never samples the transparent /
  // bilinearly-interpolated border the backend rotation leaves at the rotated
  // rectangle's edges (visible as dark corners on JPEG output). Scale both sides
  // by a common factor to preserve the requested aspect ratio exactly.
  const INSET = 1;
  let w = rect.width;
  let h = rect.height;
  const k = Math.max(0, Math.min((w - 2 * INSET) / w, (h - 2 * INSET) / h));
  w *= k;
  h *= k;

  return {
    x: (bbW - w) / 2,
    y: (bbH - h) / 2,
    width: w,
    height: h,
  };
}

/** A rectangle in screen/container coordinates (crop box representation). */
export interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

/**
 * Clamp the center-offset `(dxo, dyo)` (screen frame, relative to the shared
 * center) of an axis-aligned box with half-extents `a`/`b` so the box stays
 * inside a rectangle rotated by θ (`cos`/`sin`) with half-extents `hw`/`hh`.
 *
 * The valid-center region is itself a rectangle rotated by θ with half-extents
 * (Ku, Kv) — the Minkowski difference of the two rectangles — because the box's
 * support along the rotated axes is `a·|cosθ| + b·|sinθ|` and `a·|sinθ| +
 * b·|cosθ|`. So clamping is transform → per-axis clamp → inverse transform:
 * O(1), exact, and continuous (kink-free sliding along the boundary), unlike a
 * path-dependent binary search. Returns the clamped offset in the screen frame.
 *
 * For θ = 0 this degenerates to the classic axis-aligned clamp
 * (|dx| ≤ hw − a, |dy| ≤ hh − b).
 */
export function clampCenterInRotatedRect(
  dxo: number,
  dyo: number,
  a: number,
  b: number,
  cos: number,
  sin: number,
  hw: number,
  hh: number,
): { x: number; y: number } {
  const ac = Math.abs(cos);
  const as = Math.abs(sin);
  const Ku = Math.max(0, hw - a * ac - b * as);
  const Kv = Math.max(0, hh - a * as - b * ac);

  let u = dxo * cos + dyo * sin;
  let v = -dxo * sin + dyo * cos;
  u = Math.min(Ku, Math.max(-Ku, u));
  v = Math.min(Kv, Math.max(-Kv, v));

  return { x: u * cos - v * sin, y: u * sin + v * cos };
}

/**
 * Largest `t` in [0, 1] such that the linearly interpolated box
 * `box(t) = box0 + t·(box1 − box0)` stays inside the rotated rectangle
 * (half-extents `hw`/`hh` about center `cx`/`cy`, rotation `cos`/`sin`) and
 * keeps `width`/`height ≥ minSize`.
 *
 * `box0` must already be valid (t = 0 always fits). Each constraint has the form
 * `|linear(t)| + linear(t) ≤ limit`, which splits into two linear inequalities in
 * `t`; the feasible set is therefore `[0, t_max]` and `t_max` is a closed-form
 * minimum of upper bounds. This is O(1), exact, and continuous in the drag delta,
 * so a resize handle glides smoothly to the boundary instead of snapping between
 * "fully accepted" and "fully rejected".
 */
export function maxResizeT(
  box0: Box,
  box1: Box,
  cx: number,
  cy: number,
  cos: number,
  sin: number,
  hw: number,
  hh: number,
  minSize = 10,
): number {
  const ac = Math.abs(cos);
  const as = Math.abs(sin);

  // Center offsets in the screen frame (linear in t via the endpoints).
  const ox0 = box0.left + box0.width / 2 - cx;
  const oy0 = box0.top + box0.height / 2 - cy;
  const ox1 = box1.left + box1.width / 2 - cx;
  const oy1 = box1.top + box1.height / 2 - cy;

  // Rotated-frame center components: u(t) = u0 + t·du, v(t) = v0 + t·dv.
  const u0 = ox0 * cos + oy0 * sin;
  const u1 = ox1 * cos + oy1 * sin;
  const v0 = -ox0 * sin + oy0 * cos;
  const v1 = -ox1 * sin + oy1 * cos;
  const du = u1 - u0;
  const dv = v1 - v0;

  // Support terms: p(t) = a·ac + b·as, q(t) = a·as + b·ac (linear in t).
  const p0 = (box0.width * ac + box0.height * as) / 2;
  const p1 = (box1.width * ac + box1.height * as) / 2;
  const q0 = (box0.width * as + box0.height * ac) / 2;
  const q1 = (box1.width * as + box1.height * ac) / 2;
  const dp = p1 - p0;
  const dq = q1 - q0;

  // Solve t·A ≤ B for the upper bound on t (B ≥ 0 at t = 0 since box0 fits;
  // when A ≤ 0 the constraint never binds for t ≥ 0).
  const bound = (A: number, B: number) => (A <= 1e-9 ? Infinity : B / A);

  let t = 1;
  t = Math.min(t, bound(du + dp, hw - u0 - p0)); // u(t) + p(t) ≤ hw
  t = Math.min(t, bound(-du + dp, hw + u0 - p0)); // -u(t) + p(t) ≤ hw
  t = Math.min(t, bound(dv + dq, hh - v0 - q0)); // v(t) + q(t) ≤ hh
  t = Math.min(t, bound(-dv + dq, hh + v0 - q0)); // -v(t) + q(t) ≤ hh

  const dw = box1.width - box0.width;
  const dh = box1.height - box0.height;
  if (dw < -1e-9) t = Math.min(t, (box0.width - minSize) / -dw);
  if (dh < -1e-9) t = Math.min(t, (box0.height - minSize) / -dh);

  return Math.min(1, Math.max(0, t));
}
