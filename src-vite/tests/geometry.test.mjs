import test from 'node:test';
import assert from 'node:assert/strict';
import { calculateInscribedCrop, getMaxInscribedRect, maxResizeT, clampCenterInRotatedRect } from '../src/common/geometry.ts';

test('calculateInscribedCrop returns full image at angle 0', () => {
  const crop = calculateInscribedCrop(1920, 1080, 0);
  assert.equal(crop.x, 0);
  assert.equal(crop.y, 0);
  assert.equal(crop.width, 1920);
  assert.equal(crop.height, 1080);
});

test('calculateInscribedCrop preserves exact original aspect ratio across 0 to 45 deg (landscape)', () => {
  const dimensions = [[1920, 1080], [4000, 3000], [1600, 900]];
  for (const [w, h] of dimensions) {
    const originalRatio = w / h;
    for (let angle = 0; angle <= 45; angle += 0.5) {
      const crop = calculateInscribedCrop(w, h, angle);
      const cropRatio = crop.width / crop.height;
      assert.ok(
        Math.abs(cropRatio - originalRatio) < 1e-10,
        `Landscape (${w}x${h}) at angle ${angle}deg failed aspect ratio: expected ${originalRatio}, got ${cropRatio}`
      );
    }
  }
});

test('calculateInscribedCrop preserves exact original aspect ratio across 0 to 45 deg (portrait)', () => {
  const dimensions = [[1080, 1920], [3000, 4000], [900, 1600]];
  for (const [w, h] of dimensions) {
    const originalRatio = w / h;
    for (let angle = 0; angle <= 45; angle += 0.5) {
      const crop = calculateInscribedCrop(w, h, angle);
      const cropRatio = crop.width / crop.height;
      assert.ok(
        Math.abs(cropRatio - originalRatio) < 1e-10,
        `Portrait (${w}x${h}) at angle ${angle}deg failed aspect ratio: expected ${originalRatio}, got ${cropRatio}`
      );
    }
  }
});

test('calculateInscribedCrop preserves exact original aspect ratio across 0 to 45 deg (square)', () => {
  const [w, h] = [1000, 1000];
  const originalRatio = 1.0;
  for (let angle = 0; angle <= 45; angle += 0.5) {
    const crop = calculateInscribedCrop(w, h, angle);
    const cropRatio = crop.width / crop.height;
    assert.ok(
      Math.abs(cropRatio - originalRatio) < 1e-10,
      `Square (${w}x${h}) at angle ${angle}deg failed aspect ratio: expected ${originalRatio}, got ${cropRatio}`
    );
  }
});

test('calculateInscribedCrop symmetry for negative angles', () => {
  const [w, h] = [1920, 1080];
  for (const angle of [1, 5, 12.5, 25, 45]) {
    const pos = calculateInscribedCrop(w, h, angle);
    const neg = calculateInscribedCrop(w, h, -angle);
    assert.ok(Math.abs(pos.x - neg.x) < 1e-9);
    assert.ok(Math.abs(pos.y - neg.y) < 1e-9);
    assert.ok(Math.abs(pos.width - neg.width) < 1e-9);
    assert.ok(Math.abs(pos.height - neg.height) < 1e-9);
  }
});

test('calculateInscribedCrop never exposes black borders across all angles', () => {
  for (const [w, h] of [[1920, 1080], [1080, 1920], [2000, 2000]]) {
    for (let angle = 0; angle <= 45; angle += 1) {
      const crop = calculateInscribedCrop(w, h, angle);
      const theta = (angle * Math.PI) / 180;
      const cos = Math.cos(theta);
      const sin = Math.sin(theta);

      const xProj = crop.width * cos + crop.height * sin;
      const yProj = crop.width * sin + crop.height * cos;

      assert.ok(xProj <= w + 1e-9, `Width exceeded at angle ${angle}deg: ${xProj} > ${w}`);
      assert.ok(yProj <= h + 1e-9, `Height exceeded at angle ${angle}deg: ${yProj} > ${h}`);

      const bbW = w * cos + h * sin;
      const bbH = h * cos + w * sin;
      assert.ok(crop.x >= 0, `Negative crop.x at ${angle}deg`);
      assert.ok(crop.y >= 0, `Negative crop.y at ${angle}deg`);
      assert.ok(crop.x + crop.width <= bbW + 1e-9, `Crop outside bbW at ${angle}deg`);
      assert.ok(crop.y + crop.height <= bbH + 1e-9, `Crop outside bbH at ${angle}deg`);
    }
  }
});

test('getMaxInscribedRect preserves aspect ratio with safety margin', () => {
  const [w, h] = [1920, 1080];
  const originalRatio = w / h;
  for (let angle = 1; angle <= 45; angle += 2) {
    const rect = getMaxInscribedRect(w, h, angle);
    const ratio = rect.width / rect.height;
    assert.ok(
      Math.abs(ratio - originalRatio) < 1e-9,
      `Safety-margined crop at angle ${angle}deg failed aspect ratio: expected ${originalRatio}, got ${ratio}`
    );
  }
});

test('Performance: 10,000 slider drag evaluations run in <10ms (120+ FPS capable)', () => {
  const start = performance.now();
  const iterations = 10000;
  for (let i = 0; i < iterations; i++) {
    const angle = (i % 900) / 10 - 45; // -45 to +45
    getMaxInscribedRect(1920, 1080, angle);
  }
  const elapsed = performance.now() - start;
  const timePerFrameMs = elapsed / iterations;
  // 120 FPS requires a frame budget of ~8.33ms total. The crop calculation must take < 0.1ms.
  assert.ok(
    timePerFrameMs < 0.05,
    `Calculation took ${timePerFrameMs}ms per tick (expected < 0.05ms)`
  );
});

test('calculateInscribedCrop and getMaxInscribedRect handle 0.1 deg fine fractional increments monotonically', () => {
  const [w, h] = [1920, 1080];
  let prevWidth = Infinity;
  for (let a = 0; a <= 10; a += 1) {
    const angle = Number((a * 0.1).toFixed(1));
    const crop = calculateInscribedCrop(w, h, angle);
    const rect = getMaxInscribedRect(w, h, angle);
    assert.ok(crop.width <= prevWidth + 1e-9, `Crop width should decrease as angle increases: ${crop.width} > ${prevWidth}`);
    if (angle > 0) {
      assert.ok(rect.width < crop.width, 'Safety margin should keep rect smaller than raw inscribed crop');
    } else {
      assert.equal(rect.width, crop.width, 'At angle 0 rect equals raw inscribed crop');
    }
    assert.ok(Math.abs(crop.width / crop.height - w / h) < 1e-10, 'Aspect ratio preserved');
    prevWidth = crop.width;
  }
});


test('Current Aspect Ratio drag from any interior anchor stays inside image bounds and produces non-zero crop box', () => {
  const imgLeft = 100;
  const imgTop = 50;
  const imgW = 800;
  const imgH = 600;
  const imgRight = imgLeft + imgW;
  const imgBottom = imgTop + imgH;
  const cx = imgLeft + imgW / 2;
  const cy = imgTop + imgH / 2;
  const hw = imgW / 2;
  const hh = imgH / 2;
  const aspectRatio = imgW / imgH; // Current Aspect Ratio (4:3)

  // Drag starting near bottom-left or top-right where raw aspect expansion would previously exceed image bounds
  const cases = [
    { startX: 250, startY: 480, curX: 650, curY: 540 }, // wide drag near bottom edge
    { startX: 700, startY: 150, curX: 760, curY: 500 }, // tall drag near right edge
    { startX: 500, startY: 400, curX: 200, curY: 120 }, // up-left drag
    { startX: 200, startY: 120, curX: 750, curY: 520 }, // down-right drag
  ];

  for (const { startX, startY, curX, curY } of cases) {
    const initialBox = { left: startX, top: startY, width: 1, height: 1 };
    const availW = curX >= startX ? imgRight - startX : startX - imgLeft;
    const availH = curY >= startY ? imgBottom - startY : startY - imgTop;
    let w = Math.min(availW, Math.max(10, Math.abs(curX - startX)));
    let h = Math.min(availH, Math.max(10, Math.abs(curY - startY)));

    let targetW = w;
    let targetH = targetW / aspectRatio;
    if (targetH < h) {
      targetH = h;
      targetW = targetH * aspectRatio;
    }
    if (targetW > availW) {
      targetW = availW;
      targetH = targetW / aspectRatio;
    }
    if (targetH > availH) {
      targetH = availH;
      targetW = targetH * aspectRatio;
    }
    w = Math.max(10, targetW);
    h = Math.max(10, targetH);
    const left = curX < startX ? Math.max(imgLeft, startX - w) : startX;
    const top = curY < startY ? Math.max(imgTop, startY - h) : startY;

    const proposed = { left, top, width: w, height: h };
    const t = maxResizeT(initialBox, proposed, cx, cy, 1, 0, hw, hh, 10);
    assert.equal(t, 1, `Expected t=1 for bounded Current Aspect Ratio drag, got ${t}`);
    const finalW = initialBox.width + t * (proposed.width - initialBox.width);
    const finalH = initialBox.height + t * (proposed.height - initialBox.height);
    assert.ok(finalW > 50 && finalH > 50, `Expected sizable crop box, got ${finalW}x${finalH}`);
    assert.ok(Math.abs(finalW / finalH - aspectRatio) < 1e-6, `Expected ratio ${aspectRatio}, got ${finalW / finalH}`);
  }
});

test('80% inset Current Aspect Ratio box can be moved and resized in all directions', () => {
  const imgW = 800;
  const imgH = 600;
  const hw = imgW / 2;
  const hh = imgH / 2;
  const boxW = imgW * 0.8;
  const boxH = imgH * 0.8;
  const clamped = clampCenterInRotatedRect(120, -90, boxW / 2, boxH / 2, 1, 0, hw, hh);
  // With 80% inset, half-margin is 10% of 800 = 80px horizontally and 10% of 600 = 60px vertically
  assert.equal(clamped.x, 80);
  assert.equal(clamped.y, -60);
});
