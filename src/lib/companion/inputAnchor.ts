import { invoke } from '@tauri-apps/api/core';

type Rect = { left: number; top: number; right: number; bottom: number };

/** Measure the displayed canvases, excluding the transparent window reserve. */
export function measureInputAnchor(canvases: Array<HTMLCanvasElement | null>) {
  const width = window.innerWidth;
  const height = window.innerHeight;
  const visible: Rect[] = [];
  for (const canvas of canvases) {
    if (!canvas) continue;
    const style = getComputedStyle(canvas);
    if (style.display === 'none' || style.visibility === 'hidden' || parseFloat(style.opacity) === 0) continue;
    const rect = canvas.getBoundingClientRect();
    // Renderers clip the model to its animation area.
    const area = canvas.closest('.animation-area')?.getBoundingClientRect();
    const clipped = {
      left: Math.max(0, rect.left, area?.left ?? 0),
      top: Math.max(0, rect.top, area?.top ?? 0),
      right: Math.min(width, rect.right, area?.right ?? width),
      bottom: Math.min(height, rect.bottom, area?.bottom ?? height),
    };
    if (clipped.right > clipped.left && clipped.bottom > clipped.top) visible.push(clipped);
  }
  if (!visible.length || width <= 0 || height <= 0) return null;
  return {
    center_x: (Math.min(...visible.map(rect => rect.left)) + Math.max(...visible.map(rect => rect.right))) / 2,
    bottom: Math.max(...visible.map(rect => rect.bottom)),
    viewport_width: width,
    viewport_height: height,
  };
}

/** Independent of cursor polling, which is paused while the pet is dragged. */
export function trackInputAnchor(getCanvases: () => Array<HTMLCanvasElement | null>) {
  let previous = '';
  let pending = false;
  let destroyed = false;
  async function sync() {
    if (pending || destroyed) return;
    const anchor = measureInputAnchor(getCanvases());
    const signature = JSON.stringify(anchor);
    if (signature === previous) return;
    pending = true;
    try {
      await invoke('companion_set_input_anchor', { anchor });
      previous = signature;
    } catch {
      // Retry after a transient startup/window-switch failure.
    } finally {
      pending = false;
    }
  }
  void sync();
  const timer = setInterval(() => { void sync(); }, 250);
  return () => { destroyed = true; clearInterval(timer); };
}
