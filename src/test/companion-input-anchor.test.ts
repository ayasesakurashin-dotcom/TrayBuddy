import { afterEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ invoke: vi.fn(async () => {}) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
import { measureInputAnchor, trackInputAnchor } from '$lib/companion/inputAnchor';

function canvas(left: number, top: number, right: number, bottom: number) {
  const element = document.createElement('canvas');
  vi.spyOn(element, 'getBoundingClientRect').mockReturnValue({ left, top, right, bottom } as DOMRect);
  document.body.append(element);
  return element;
}

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
  vi.clearAllMocks();
  vi.useRealTimers();
});

describe('input follows the displayed pet instead of its transparent window', () => {
  it('reports the model bottom with hundreds of pixels still below it in the window', () => {
    const model = canvas(100, 300, 400, 540);
    expect(measureInputAnchor([model, null])).toEqual({
      center_x: 250, bottom: 540,
      viewport_width: window.innerWidth, viewport_height: window.innerHeight,
    });
  });

  it('includes a visible border, then follows the model when the border is hidden', () => {
    const model = canvas(100, 300, 400, 540);
    const border = canvas(80, 500, 420, 600);
    expect(measureInputAnchor([model, border])?.bottom).toBe(600);
    border.style.visibility = 'hidden';
    expect(measureInputAnchor([model, border])?.bottom).toBe(540);
    model.style.display = 'none';
    expect(measureInputAnchor([model, border])).toBeNull();
  });

  it('uses only the part of a large model inside the clipped animation area', () => {
    const area = document.createElement('div');
    area.className = 'animation-area';
    vi.spyOn(area, 'getBoundingClientRect').mockReturnValue({ left: 0, top: 300, right: 500, bottom: 560 } as DOMRect);
    const model = canvas(-50, 250, 550, 650);
    area.append(model);
    document.body.append(area);
    expect(measureInputAnchor([model])).toMatchObject({ center_x: 250, bottom: 560 });
  });

  it('tracks layout changes during dragging, deduplicates updates and stops on cleanup', async () => {
    vi.useFakeTimers();
    const model = canvas(100, 300, 400, 540);
    const stop = trackInputAnchor(() => [model]);
    await vi.advanceTimersByTimeAsync(250);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    vi.mocked(model.getBoundingClientRect).mockReturnValue({ left: 150, top: 300, right: 450, bottom: 600 } as DOMRect);
    await vi.advanceTimersByTimeAsync(250);
    expect(mocks.invoke).toHaveBeenLastCalledWith('companion_set_input_anchor', {
      anchor: { center_x: 300, bottom: 600, viewport_width: window.innerWidth, viewport_height: window.innerHeight },
    });
    stop();
    await vi.advanceTimersByTimeAsync(1000);
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
  });

  it('retries a failed update and clears the anchor when the pet disappears', async () => {
    vi.useFakeTimers();
    mocks.invoke.mockRejectedValueOnce(new Error('window is not ready'));
    const model = canvas(100, 300, 400, 540);
    const stop = trackInputAnchor(() => [model]);
    await vi.advanceTimersByTimeAsync(250);
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
    model.style.visibility = 'hidden';
    await vi.advanceTimersByTimeAsync(250);
    expect(mocks.invoke).toHaveBeenLastCalledWith('companion_set_input_anchor', { anchor: null });
    stop();
  });
});
