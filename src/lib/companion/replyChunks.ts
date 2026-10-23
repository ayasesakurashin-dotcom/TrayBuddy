/** Keep companion pages small enough to fit beside the character at normal scale. */
export const REPLY_CHUNK_MAX_CHARS = 80;
export const REPLY_READ_HOLD_MS = 4000;
export const REPLY_STREAM_FLUSH_MS = 600;
const MIN_STREAM_PAGE_CHARS = 8;

const graphemes = new Intl.Segmenter(undefined, { granularity: 'grapheme' });
const sentenceEnd = /[。！？!?；;\n]/u;
const closingPunctuation = /[”’」』）》】"']/u;

/** Links are displayed as labels, never as model-generated clickable URLs. */
function plainReplyText(text: string, done: boolean): string {
  const plain = text.replace(/\[([^\]]+)\]\([^)]+\)/g, '$1');
  // Withhold packet-spanning links rather than showing a prefix that would
  // later need to be erased when the URL is replaced by its label.
  if (!done) return plain.replace(/\[[^\]]*(?:\](?:\([^)]*)?)?$/u, '');
  return plain;
}

/** A cumulative stream cursor. Reading a page never discards the unshown tail. */
export class ReplyChunkBuffer {
  private received = '';
  private text = '';
  private offset = 0;
  done = false;

  append(text: string, done: boolean): boolean {
    if (this.done || !text.startsWith(this.received)) return false;
    const plain = plainReplyText(text, done);
    if (!plain.startsWith(this.text)) return false;
    const changed = text !== this.received || done;
    this.received = text;
    this.text = plain;
    this.done = done;
    return changed;
  }

  get hasPending(): boolean { return this.offset < this.text.length; }

  take(force = false): string | null {
    const remaining = this.text.slice(this.offset);
    if (!remaining) return null;
    const units: string[] = [];
    let newlines = 0;
    for (const part of graphemes.segment(remaining)) {
      units.push(part.segment);
      if (part.segment.includes('\n')) newlines++;
      if (units.length === REPLY_CHUNK_MAX_CHARS || newlines === 3) break;
    }

    const fullPage = units.length === REPLY_CHUNK_MAX_CHARS || newlines === 3;
    // A timer is not evidence that a two-character SSE packet is a full page.
    // Keep unfinished prefixes such as “查询” for the following packet. A real
    // short answer is released at completion, or after sentence punctuation.
    if (!this.done && units.filter(unit => /\S/u.test(unit)).length < MIN_STREAM_PAGE_CHARS
      && !units.some(unit => /[。！？!?]/u.test(unit))) return null;
    let length = units.length;
    if (fullPage || (!this.done && !force)) {
      // Prefer a sentence, then a word boundary. Avoid a one-word first page
      // when more text is available, while still starting long streams early.
      const minBoundary = fullPage ? 24 : 12;
      let sentence = 0;
      let word = 0;
      for (let index = 0; index < units.length; index++) {
        if (sentenceEnd.test(units[index]) || (units[index] === '.' && /\s/u.test(units[index + 1] ?? ''))) {
          let end = index + 1;
          while (end < units.length && closingPunctuation.test(units[end])) end++;
          if (end >= minBoundary) sentence = end;
        }
        if (/\s/u.test(units[index]) && index + 1 >= minBoundary) word = index + 1;
      }
      if (sentence) length = sentence;
      else if (fullPage) length = word || units.length;
      else return null;
    }

    const chunk = units.slice(0, length).join('');
    this.offset += chunk.length;
    return chunk;
  }
}
