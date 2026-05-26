/** IRC message parser and formatter. */
import type { IRCMessage } from './types.js';
/** Extract nick from a prefix like "nick!user@host". */
export declare function prefixNick(prefix: string): string;
/** Parse a raw IRC line into a structured message. */
export declare function parse(raw: string): IRCMessage;
/** Format a raw IRC line from parts. */
export declare function format(command: string, params: string[], tags?: Record<string, string>): string;
//# sourceMappingURL=parser.d.ts.map