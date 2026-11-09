import { invoke } from '@tauri-apps/api/core';
import { config } from '@/common/config';

/**
 * UI interaction activity sensor.
 * Listens to scroll, mouse, and key events and notifies the backend cooperative scheduler
 * so that background tasks (face indexing, scanner, thumbnail batching) immediately yield
 * CPU and disk bandwidth, maintaining smooth 60-120 FPS UI frame delivery during interaction.
 */

let lastReportTime = 0;
const THROTTLE_MS = 60;
let isSensorInitialized = false;

export function reportInteraction(source: string = 'generic') {
  if (config.settings.processLibraryInBackground === false) return;
  const now = performance.now();
  // Throttle IPC invocations to avoid channel congestion while keeping backend lease active
  if (now - lastReportTime >= THROTTLE_MS) {
    lastReportTime = now;
    void invoke('report_ui_interaction', { source }).catch(() => {});
  }
}

export function setContinuousInteraction(active: boolean) {
  void invoke('set_ui_interaction_active', { active }).catch(() => {});
}

export function initUiInteractionSensor() {
  if (isSensorInitialized || typeof window === 'undefined') return;
  isSensorInitialized = true;

  const onWheel = () => reportInteraction('wheel');
  const onScroll = () => reportInteraction('scroll');
  const onPointerMove = (e: PointerEvent) => {
    if (e.buttons > 0) {
      reportInteraction('drag');
    } else {
      reportInteraction('pointermove');
    }
  };
  const onKeyDown = () => reportInteraction('key');
  const onTouchMove = () => reportInteraction('touch');

  // Passive capture listeners for zero-overhead event interception
  window.addEventListener('wheel', onWheel, { passive: true, capture: true });
  window.addEventListener('scroll', onScroll, { passive: true, capture: true });
  window.addEventListener('pointermove', onPointerMove, { passive: true, capture: true });
  window.addEventListener('keydown', onKeyDown, { passive: true, capture: true });
  window.addEventListener('touchmove', onTouchMove, { passive: true, capture: true });
}
