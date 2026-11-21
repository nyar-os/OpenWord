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
    document_find(handle: number, query: string, limit?: number): string;
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
