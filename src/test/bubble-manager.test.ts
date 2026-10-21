import { render } from "@testing-library/svelte";
import { tick } from "svelte";
import { describe, expect, it, vi } from "vitest";
import BubbleManager from "$lib/bubble/BubbleManager.svelte";
import { BUBBLE_SWITCH_DELAY_MS } from "$lib/constants";

describe("BubbleManager", () => {
  it('cancels a pending switch when hidden and never restores stale text', async () => {
    vi.useFakeTimers();
    const { component } = render(BubbleManager);
    component.show({ text: 'First' });
    component.show({ text: 'Late' });
    component.hide();
    await vi.advanceTimersByTimeAsync(BUBBLE_SWITCH_DELAY_MS + 10);
    expect(component.isShowing()).toBe(false);
    component.show({ text: 'Current' });
    await vi.advanceTimersByTimeAsync(BUBBLE_SWITCH_DELAY_MS + 10);
    expect(component.isShowing()).toBe(true);
    vi.useRealTimers();
  });
  it("shows, hides, and switches bubbles", async () => {
    vi.useFakeTimers();
    const { component } = render(BubbleManager);

    await tick();
    await Promise.resolve();
    await tick();

    expect(component.isShowing()).toBe(false);

    component.show({ text: "Hello" });
    expect(component.isShowing()).toBe(true);

    component.show({ text: "Next" });
    vi.advanceTimersByTime(BUBBLE_SWITCH_DELAY_MS + 10);
    await tick();
    expect(component.isShowing()).toBe(true);

    component.hide();
    expect(component.isShowing()).toBe(false);

    vi.useRealTimers();
  });
});
