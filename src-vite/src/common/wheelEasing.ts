// Independently reconstructed from the Picasa wheel-motion observations.
// Evidence and external contracts: C:/Data/lap052/picasa-wheel-re/findings.json.
const rawEase = (x: number) => x < 1
  ? x - 1 + Math.exp(-x)
  : Math.exp(-1) + (1 - Math.exp(1 - x)) * (1 - Math.exp(-1));
const normalization = 1 / rawEase(8);

export function wheelEase(t: number): number {
  return t <= 0 ? 0 : t >= 1 ? 1 : rawEase(t * 8) * normalization;
}

type Segment = { delta: number; start: number; duration: number };

export class WheelMotion {
  private base = 0;
  private segments: Segment[] = [];

  get active() { return this.segments.length > 0; }

  reset(position: number) {
    this.base = position;
    this.segments = [];
  }

  sample(now: number, maximum: number): number {
    const remaining: Segment[] = [];
    let position = this.base;
    for (const segment of this.segments) {
      const t = (now - segment.start) / segment.duration;
      position += segment.delta * wheelEase(t);
      if (t >= 1) this.base += segment.delta;
      else remaining.push(segment);
    }
    this.segments = remaining;
    const bounded = Math.max(0, Math.min(maximum, position));
    if (bounded !== position) this.reset(bounded);
    return bounded;
  }

  add(delta: number, now: number, maximum: number, shift = false) {
    const position = this.sample(now, maximum);
    // Reverse promptly instead of retaining motion towards the old destination.
    // The original application's reversal policy was not dynamically verified.
    if (this.segments.length && Math.sign(delta) !== Math.sign(this.segments.at(-1)!.delta)) {
      this.reset(position);
    }
    const destination = this.base + this.segments.reduce((sum, s) => sum + s.delta, 0);
    const bounded = Math.max(0, Math.min(maximum, destination + delta));
    if (bounded !== destination) {
      this.segments.push({ delta: bounded - destination, start: now + 10, duration: shift ? 450 : 900 });
    }
  }
}

// WheelEvent does not expose its physical device. Only recognizable discrete
// detents are eased; small/fractional/diagonal touchpad input stays native.
export function wheelDistance(event: Pick<WheelEvent, 'deltaX' | 'deltaY' | 'deltaMode' | 'ctrlKey' | 'metaKey' | 'altKey'>): number | null {
  if (event.ctrlKey || event.metaKey || event.altKey || event.deltaX || !Number.isFinite(event.deltaY)) return null;
  const amount = Math.abs(event.deltaY);
  if (!amount) return null;
  if (event.deltaMode === 1 && Number.isInteger(amount)) return event.deltaY * 40;
  if (event.deltaMode !== 0) return null;
  const unit = amount % 100 === 0 ? 100 : amount % 120 === 0 ? 120 : 0;
  return unit ? event.deltaY / unit * 120 : null;
}

export function installWheelEasing(element: HTMLElement) {
  const motion = new WheelMotion();
  let frame = 0;
  let lastWritten = element.scrollTop;
  const maximum = () => Math.max(0, element.scrollHeight - element.clientHeight);
  const cancel = () => {
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    lastWritten = element.scrollTop;
    motion.reset(lastWritten);
  };
  const tick = (now: number) => {
    frame = 0;
    // A jump outside this controller must never be pulled back by old motion.
    if (Math.abs(element.scrollTop - lastWritten) > 1) { cancel(); return; }
    element.scrollTop = motion.sample(now, maximum());
    lastWritten = element.scrollTop;
    if (motion.active) frame = requestAnimationFrame(tick);
  };
  const wheel = (event: WheelEvent) => {
    const distance = wheelDistance(event);
    if (event.defaultPrevented || !event.cancelable || distance === null) { cancel(); return; }
    event.preventDefault();
    if (!motion.active || Math.abs(element.scrollTop - lastWritten) > 1) {
      cancel();
      // Take ownership from any native touchpad/keyboard animation before the
      // first frame; otherwise it can look like a jump and cancel this detent.
      element.scrollTo({ top: lastWritten, behavior: 'instant' });
    }
    motion.add(distance, performance.now(), maximum(), event.shiftKey);
    if (motion.active && !frame) frame = requestAnimationFrame(tick);
  };
  const key = (event: KeyboardEvent) => {
    if (['ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End', ' '].includes(event.key)) cancel();
  };
  element.addEventListener('wheel', wheel, { passive: false });
  element.addEventListener('pointerdown', cancel);
  element.addEventListener('keydown', key);
  return {
    cancel,
    destroy() {
      cancel();
      element.removeEventListener('wheel', wheel);
      element.removeEventListener('pointerdown', cancel);
      element.removeEventListener('keydown', key);
    },
  };
}
