import { currentMonitor } from '@tauri-apps/api/window';

// Keep initial size and subsequent UI-scale changes consistent across windows.
export async function settingsWindowGeometry(scale: number) {
  const monitor = await currentMonitor();
  const factor = monitor?.scaleFactor || 1;
  const availableWidth = monitor ? monitor.workArea.size.width / factor : 1200;
  const availableHeight = monitor ? monitor.workArea.size.height / factor : 900;
  const width = Math.min(Math.round(900 * scale), Math.max(320, availableWidth - 40));
  const height = Math.min(Math.round(600 * scale), Math.max(320, availableHeight - 40));
  return {
    width, height,
    minWidth: Math.min(Math.round(600 * scale), width),
    minHeight: Math.min(Math.round(440 * scale), height),
    ...(monitor ? {
      x: Math.round(monitor.workArea.position.x / factor + (availableWidth - width) / 2),
      y: Math.round(monitor.workArea.position.y / factor + (availableHeight - height) / 2),
    } : {}),
  };
}
