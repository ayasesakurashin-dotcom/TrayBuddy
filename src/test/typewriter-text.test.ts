import { render, fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";
import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import TypewriterText from "$lib/bubble/TypewriterText.svelte";


describe("TypewriterText", () => {
  it('completes a companion page only once without loading constants or restarting after mount', async () => {
    vi.useFakeTimers();
    const complete = vi.fn();
    const invokeMock = vi.mocked(invoke);
    const firstCall = invokeMock.mock.calls.length;
    const { container, unmount } = render(TypewriterText, {
      props: { text: '完整回复', plainText: true, speed: 10 },
      events: { complete },
    });
    await tick();
    await vi.advanceTimersByTimeAsync(100);
    expect(complete).toHaveBeenCalledOnce();
    expect(container.querySelector('.typewriter-text')?.textContent).toBe('完整回复');
    await vi.advanceTimersByTimeAsync(10000);
    await tick();
    expect(complete).toHaveBeenCalledOnce();
    expect(container.querySelector('.cursor')).toBeNull();
    expect(invokeMock.mock.calls.slice(firstCall).some(call => call[0] === 'get_const_int')).toBe(false);
    unmount();
    vi.useRealTimers();
  });
  it('keeps a clicked-to-finish companion page complete through later mount microtasks', async () => {
    vi.useFakeTimers();
    const complete = vi.fn();
    const {container, unmount} = render(TypewriterText, {
      props: {text: '点击后不应从头播放', plainText: true, speed: 1000},
      events: {complete},
    });
    await fireEvent.click(container.querySelector('.typewriter-text')!);
    await vi.advanceTimersByTimeAsync(10000);
    await tick();
    expect(complete).toHaveBeenCalledOnce();
    expect(container.querySelector('.typewriter-text')?.textContent).toBe('点击后不应从头播放');
    expect(container.querySelector('.cursor')).toBeNull();
    unmount();
    vi.useRealTimers();
  });
  it('keeps companion text literal and wraps in CSS without adding line breaks', async () => {
    const text = '[链接](https://example.com) **字面文字** ' + '中文'.repeat(30) + '👩‍💻';
    const { container } = render(TypewriterText, { props: { text, instant: true, plainText: true } });
    await tick();
    expect(container.querySelector('.typewriter-text')?.textContent).toBe(text);
    expect(container.querySelector('a, strong, br')).toBeNull();
  });
  it('types a Unicode code point without exposing a surrogate half', async () => {
    vi.useFakeTimers();
    const { container } = render(TypewriterText, { props: { text: '😀好', plainText: true, speed: 10 } });
    await tick();
    vi.advanceTimersByTime(10);
    await tick();
    expect(container.querySelector('.typewriter-text')?.textContent).toBe('😀|');
    vi.useRealTimers();
  });
  it("types text and completes", async () => {
    vi.useFakeTimers();
    const { container } = render(TypewriterText, {
      props: { text: "AB", speed: 10 },
    });

    await tick();
    await Promise.resolve();
    await tick();

    vi.advanceTimersByTime(1000);
    await tick();
    await Promise.resolve();
    await tick();

    expect(container.querySelector(".cursor")).toBeFalsy();
    expect(container.querySelector(".typewriter-text")?.classList.contains("complete")).toBe(true);

    vi.useRealTimers();
  });

  it("click skips typing and shows full text", async () => {
    vi.useFakeTimers();
    const { container } = render(TypewriterText, {
      props: { text: "Hello", speed: 1000 },
    });

    await tick();
    const el = container.querySelector(".typewriter-text") as HTMLElement | null;
    await fireEvent.click(el as HTMLElement);
    await tick();

    expect(container.querySelector(".cursor")).toBeFalsy();
    expect(container.querySelector(".typewriter-text")?.classList.contains("complete")).toBe(true);

    vi.useRealTimers();
  });

  it("renders instantly when instant is true", async () => {
    const { container } = render(TypewriterText, {
      props: { text: "Instant", instant: true },
    });

    await tick();

    expect(container.querySelector(".cursor")).toBeFalsy();
    expect(container.querySelector(".typewriter-text")?.classList.contains("complete")).toBe(true);
  });

  it("wraps long text when max_chars_per_line is provided", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockResolvedValueOnce({ max_chars_per_line: 2 });

    const { container, rerender } = render(TypewriterText, {
      props: { text: "ABCD", instant: true },
    });

    await tick();
    await rerender({ text: "ABCDE", instant: true });
    await tick();

    const html = container.querySelector(".typewriter-text")?.innerHTML ?? "";
    expect(html).toContain("<br>");
  });


  it("does not wrap when max_chars_per_line is non-positive", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockResolvedValueOnce({ max_chars_per_line: -1 });

    const { container } = render(TypewriterText, {
      props: { text: "ABCD", instant: true },
    });

    await tick();

    const html = container.querySelector(".typewriter-text")?.innerHTML ?? "";
    expect(html).not.toContain("<br>");
  });
});

