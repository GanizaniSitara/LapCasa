export type CurvePoint = [number, number];
export type CurveChannel = 'rgb' | 'red' | 'green' | 'blue';
export type Curves = Record<CurveChannel, CurvePoint[]>;
export const curveChannels: CurveChannel[] = ['rgb', 'red', 'green', 'blue'];
export const identityCurves = (): Curves => Object.fromEntries(curveChannels.map(c => [c, [[0, 0], [1, 1]]])) as Curves;

// Matches t_curves.rs: monotone cubic Hermite segments with bounded tangents.
export function curveLut(points: CurvePoint[]): number[] {
  const d = points.slice(1).map((p, i) => (p[1] - points[i][1]) / (p[0] - points[i][0]));
  const m = points.map(() => 0);
  m[0] = d[0]; m[m.length - 1] = d[d.length - 1];
  for (let i = 1; i < m.length - 1; i++) {
    if (d[i - 1] * d[i] > 0) {
      const left = points[i][0] - points[i - 1][0], right = points[i + 1][0] - points[i][0];
      const a = 2 * right + left, b = right + 2 * left;
      m[i] = (a + b) / (a / d[i - 1] + b / d[i]);
    }
  }
  for (let i = 0; i < d.length; i++) {
    if (d[i] === 0) { m[i] = m[i + 1] = 0; continue; }
    const norm = (m[i] / d[i]) ** 2 + (m[i + 1] / d[i]) ** 2;
    if (norm > 9) { const factor = 3 / Math.sqrt(norm); m[i] *= factor; m[i + 1] *= factor; }
  }
  return Array.from({ length: 256 }, (_, value) => {
    const x = value / 255;
    const i = Math.max(0, points.findIndex((p, i) => i > 0 && x <= p[0]) - 1);
    const h = points[i + 1][0] - points[i][0], t = (x - points[i][0]) / h;
    const y = (2 * t ** 3 - 3 * t * t + 1) * points[i][1]
      + (t ** 3 - 2 * t * t + t) * h * m[i]
      + (-2 * t ** 3 + 3 * t * t) * points[i + 1][1]
      + (t ** 3 - t * t) * h * m[i + 1];
    return Math.round(Math.max(0, Math.min(1, y)) * 255);
  });
}

export function applyCurves(pixels: Uint8ClampedArray, curves: Curves) {
  const tables = curveChannels.map(c => curveLut(curves[c]));
  for (let i = 0; i < pixels.length; i += 4) {
    for (let c = 0; c < 3; c++) pixels[i + c] = tables[c + 1][tables[0][pixels[i + c]]];
  }
}
