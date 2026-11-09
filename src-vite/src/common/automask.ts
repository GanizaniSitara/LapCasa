// Automask (LAP-107): the sliders' bend, as slider_curve in t_edit_local.rs, for the curve the editor
// draws under the editable points. Auto and Auto Balance are computed in t_edit_balance.rs.

/** The sliders' bend of x (0..1), each slider -1..1; identity when all are 0. */
export function bend(x: number, s: number, h: number, w: number, b: number) {
  const x1 = 1 - x;
  return x + s * 0.3 * 6.75 * x * x1 * x1 + h * 0.3 * 6.75 * x * x * x1 + b * 0.12 * x1 ** 4 + w * 0.12 * x ** 4;
}
