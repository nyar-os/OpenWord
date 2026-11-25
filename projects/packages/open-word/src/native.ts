/**
 * Native addon discovery.
 *
 * The addon only maps values: document semantics, saving and export stay in the
 * shared engine. An absent addon is a declarative state, so capability calls
 * report `capability/not-implemented` instead of inventing a matrix.
 */

import { createRequire } from 'node:module';

const requireNative = createRequire(import.meta.url);

/** Orthogonal exports of the built addon. Document calls return JSON; close returns a boolean. */
export interface NativeBinding {
    version(): string;
    capabilities(): string;
    document_create(requestJson: string): string;
    document_open(path: string): string;
    document_save(handle: number, path: string, overwrite: boolean): string;
    document_inspect(handle: number, checksJson?: string): string;
    document_summary(handle: number): string;
    document_outline(handle: number): string;
    word_stats(handle: number): string;
    document_find(handle: number, query: string, limit?: number): string;
    document_replace(handle: number, requestJson: string): string;
    document_edit(handle: number, requestJson: string): string;
    document_undo(handle: number, steps?: number): string;
    document_redo(handle: number, steps?: number): string;
    document_merge(handle: number, path: string, mode?: string): string;
    document_export(handle: number, format: string, target: string, overwrite?: boolean): string;
    document_close(handle: number): boolean;
    openGui(options?: { title?: string; contentDir?: string; width?: number; height?: number }): string;
    pumpEvents(): string;
    closeGui(): string;
    isGuiOpen(): boolean;
}

let cached: NativeBinding | null | undefined;

/**
 * One failure as the addon reports it.
 *
 * `code` and `detail.params` are the machine contract; `message` is display
 * text only. `detail` also carries the optional locators beside the payload.
 */
export interface NativeFailure {
    readonly code: string;
    readonly message: string;
    readonly detail: {
        readonly params?: unknown;
        readonly revision?: number;
        readonly documentId?: string;
        readonly objectId?: string;
        readonly format?: string;
        readonly feature?: string;
        readonly recoverable?: boolean;
    };
}

/**
 * Reads one thrown native value into {@link NativeFailure}, or `undefined` when
 * it carries no stable code.
 *
 * This only reads fields: no branch compares message text.
 */
export function readNativeFailure(error: unknown): NativeFailure | undefined {
    if (typeof error !== 'object' || error === null) {
        return undefined;
    }
    const candidate = error as { code?: unknown; message?: unknown; detail?: unknown };
    if (typeof candidate.code !== 'string' || candidate.code.length === 0) {
        return undefined;
    }
    const detail = typeof candidate.detail === 'object' && candidate.detail !== null ? (candidate.detail as NativeFailure['detail']) : {};
    const message = typeof candidate.message === 'string' ? candidate.message : String(error);
    return { code: candidate.code, message, detail };
}

/** Loads the addon once, or returns `undefined` when it is not installed. */
export function nativeBinding(): NativeBinding | undefined {
    if (cached === undefined) {
        cached = load();
    }
    return cached ?? undefined;
}

function load(): NativeBinding | null {
    for (const candidate of candidates()) {
        try {
            return requireNative(candidate) as NativeBinding;
        } catch (error) {
            if (candidate === explicitPath()) {
                throw error;
            }
            if (!isModuleNotFound(error, candidate)) {
                throw error;
            }
        }
    }
    return null;
}

function candidates(): string[] {
    const explicit = explicitPath();
    if (explicit) {
        return [explicit];
    }
    return [`@wae/open-word-${process.platform}-${process.arch}`];
}

function explicitPath(): string | undefined {
    const value = process.env.OPEN_WORD_NATIVE;
    return value !== undefined && value.length > 0 ? value : undefined;
}

function isModuleNotFound(error: unknown, candidate: string): boolean {
    if (typeof error !== 'object' || error === null) {
        return false;
    }
    const code = (error as { code?: unknown }).code;
    // Node prints the resolved path, which uses the platform separator on
    // Windows. Normalize it so a linked platform package without a built
    // addon is still treated as "not installed" instead of a fatal error.
    const message = (error as Error).message.replace(/\\/g, '/');
    return code === 'MODULE_NOT_FOUND' && message.includes(candidate);
}
