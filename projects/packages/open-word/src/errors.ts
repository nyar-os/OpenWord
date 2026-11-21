/**
 * Stable error surface of `@wae/open-word`.
 *
 * Codes are contract, not prose: callers compare them, localize around them and
 * never parse the message. A failure therefore keeps its code even when the
 * text is translated.
 */

/** Every stable code the authoring surface may report. */
export const errorCodes = [
    'session/disposed',
    'session/argument-invalid',
    'session/budget-exceeded',
    'document/not-found',
    'document/revision-conflict',
    'object/not-found',
    'object/unsupported',
    'format/unsupported',
    'format/invalid',
    'capability/not-implemented',
] as const;

/** Union of {@link errorCodes}. */
export type OpenWordErrorCode = (typeof errorCodes)[number];

/** Optional locators a failure can carry so callers can point at the object. */
export interface OpenWordErrorDetails {
    readonly revision?: number;
    readonly documentId?: string;
    readonly objectId?: string;
    /** Document format involved in the failure, without the leading dot. */
    readonly format?: string;
    /** Authoring capability involved in the failure, e.g. `document.export`. */
    readonly feature?: string;
}

/** Options accepted by {@link OpenWordError}. */
export interface OpenWordErrorOptions {
    readonly recoverable?: boolean;
    readonly details?: OpenWordErrorDetails;
    readonly cause?: unknown;
}

/** Every failure thrown or rejected by this package. */
export class OpenWordError extends Error {
    /** Stable, comparable code. */
    readonly code: OpenWordErrorCode;
    /** Whether retrying with different input or state can succeed. */
    readonly recoverable: boolean;
    /** Optional locators (`revision`, `documentId`, `objectId`, `format`, `feature`). */
    readonly details: OpenWordErrorDetails;

    constructor(code: OpenWordErrorCode, message: string, options: OpenWordErrorOptions = {}) {
        super(message, options.cause === undefined ? undefined : { cause: options.cause });
        this.name = 'OpenWordError';
        this.code = code;
        this.recoverable = options.recoverable ?? false;
        this.details = Object.freeze({ ...options.details });
    }
}

/**
 * Reported by declared methods whose backing capability is not wired yet.
 *
 * The scaffold uses one code for every unimplemented entry point instead of
 * inventing per-method placeholders; each method drops this rejection as it is
 * implemented, and the code disappears with the last one.
 */
export class NotImplementedError extends OpenWordError {
    constructor(feature: string, message = `\`${feature}\` is not implemented yet`) {
        super('capability/not-implemented', message, { recoverable: false, details: { feature } });
        this.name = 'NotImplementedError';
    }
}

/** Narrows an unknown thrown value to {@link OpenWordError}. */
export function isOpenWordError(value: unknown): value is OpenWordError {
    return value instanceof OpenWordError;
}
