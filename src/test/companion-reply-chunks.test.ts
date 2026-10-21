import { describe, expect, it } from 'vitest';
import { ReplyChunkBuffer, REPLY_CHUNK_MAX_CHARS } from '$lib/companion/replyChunks';

function drain(text: string) {
  const buffer = new ReplyChunkBuffer();
  buffer.append(text, true);
  const chunks: string[] = [];
  while (buffer.hasPending) chunks.push(buffer.take()!);
  return chunks;
}

describe('companion reply pages', () => {
  it('retains every character including the last short page of an unlimited response', () => {
    const text = '这是一段包含完整句子的回复。'.repeat(400) + '最后一句';
    const chunks = drain(text);
    expect(chunks.join('')).toBe(text);
    expect(chunks.every(chunk => Array.from(chunk).length <= REPLY_CHUNK_MAX_CHARS)).toBe(true);
    expect(chunks.length).toBeGreaterThan(50);
  });

  it('splits on sentence and word boundaries instead of cutting ordinary words', () => {
    const sentence = '这是一个完整的自然语言句子，可以在标点处分成一条合适的气泡。';
    expect(drain(sentence.repeat(4))[0]).toBe(sentence.repeat(2));
    const words = 'A readable stream preserves whole words and natural spacing. '.repeat(8);
    const chunks = drain(words);
    expect(chunks.join('')).toBe(words);
    expect(chunks.slice(0, -1).every(chunk => /[\s.!?]$/u.test(chunk))).toBe(true);
  });

  it('never cuts surrogate pairs, combining marks, or emoji families in half', () => {
    const unit = '👨‍👩‍👧‍👦e\u0301🌸';
    const text = unit.repeat(60);
    const chunks = drain(text);
    const segments = new Intl.Segmenter(undefined, { granularity: 'grapheme' });
    expect(chunks.join('')).toBe(text);
    expect(chunks.every(chunk => [...segments.segment(chunk)].length <= REPLY_CHUNK_MAX_CHARS)).toBe(true);
    expect(chunks.every(chunk => !chunk.startsWith('\u200d') && !chunk.startsWith('\u0301') && !chunk.endsWith('\u200d'))).toBe(true);
  });

  it('caps explicit lines as well as text length for lists and code blocks', () => {
    const text = Array.from({length: 40}, (_, index) => `${index + 1}. 内容\n`).join('');
    const chunks = drain(text);
    expect(chunks.join('')).toBe(text);
    expect(chunks.every(chunk => (chunk.match(/\n/g) ?? []).length <= 3)).toBe(true);
  });

  it('ignores replayed and regressive packets without losing a later tail', () => {
    const buffer = new ReplyChunkBuffer();
    expect(buffer.append('第一段还没有结束', false)).toBe(true);
    expect(buffer.take()).toBeNull();
    expect(buffer.take(true)).toBe('第一段还没有结束');
    expect(buffer.append('第一段', false)).toBe(false);
    expect(buffer.append('第一段还没有结束', false)).toBe(false);
    expect(buffer.append('第一段还没有结束，尾段完整。', true)).toBe(true);
    expect(buffer.take()).toBe('，尾段完整。');
    expect(buffer.append('第一段还没有结束，尾段完整。', true)).toBe(false);
    expect(buffer.hasPending).toBe(false);
  });

  it('buffers a link spanning packets and displays its label once', () => {
    const buffer = new ReplyChunkBuffer();
    buffer.append('参考[文档', false);
    expect(buffer.take(true)).toBe('参考');
    buffer.append('参考[文档]', false);
    expect(buffer.take(true)).toBeNull();
    buffer.append('参考[文档](https://example.com', false);
    expect(buffer.take(true)).toBeNull();
    buffer.append('参考[文档](https://example.com)中的说明。', true);
    expect(buffer.take()).toBe('文档中的说明。');
  });
});
