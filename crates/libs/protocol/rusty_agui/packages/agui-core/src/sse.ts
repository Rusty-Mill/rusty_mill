// Server-Sent Events framing: one event per `data:` frame.

import { parseEvent, type Event } from "./types.js";

export const CONTENT_TYPE = "text/event-stream";

/** One event as an SSE frame. */
export function encode(event: Event): string {
  return `data: ${JSON.stringify(event)}\n\n`;
}

/** The joined `data:` payload of one frame, if it has one. */
function dataOf(frame: string): string | undefined {
  let data: string | undefined;
  for (const line of frame.split(/\r\n|\r|\n/)) {
    if (!line.startsWith("data:")) continue;
    let rest = line.slice(5);
    if (rest.startsWith(" ")) rest = rest.slice(1);
    data = data === undefined ? rest : `${data}\n${rest}`;
  }
  return data;
}

/**
 * An incremental SSE parser: feed text as it arrives, take events as
 * frames complete. Handles `\n`, `\r\n` and `\r`, multi-line `data:`,
 * comments, and ignores `event:`, `id:` and `retry:`.
 */
export class Decoder {
  private buffer = "";

  /** Appends text and returns every event whose frame is now complete. */
  feed(text: string): Event[] {
    this.buffer += text;
    const events: Event[] = [];
    for (;;) {
      const end = findBlankLine(this.buffer);
      if (end === undefined) break;
      const frame = this.buffer.slice(0, end.bodyEnd);
      this.buffer = this.buffer.slice(end.frameEnd);
      const data = dataOf(frame);
      if (data !== undefined) events.push(parseEvent(JSON.parse(data)));
    }
    return events;
  }

  /** Flushes a final frame that ended without a blank line. */
  finish(): Event | undefined {
    const rest = this.buffer;
    this.buffer = "";
    const data = dataOf(rest);
    return data === undefined ? undefined : parseEvent(JSON.parse(data));
  }
}

function terminatorAt(text: string, i: number): number | undefined {
  const c = text[i];
  if (c === "\n") return 1;
  if (c === "\r") return text[i + 1] === "\n" ? 2 : 1;
  return undefined;
}

/** The first blank line: two consecutive line terminators. */
function findBlankLine(text: string): { bodyEnd: number; frameEnd: number } | undefined {
  let i = 0;
  while (i < text.length) {
    const first = terminatorAt(text, i);
    if (first === undefined) {
      i += 1;
      continue;
    }
    const second = terminatorAt(text, i + first);
    if (second !== undefined) return { bodyEnd: i, frameEnd: i + first + second };
    i += first;
  }
  return undefined;
}
