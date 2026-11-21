/**
 * WebView host surface.
 *
 * The window is a WAE `WebViewHost` serving the built VMZ studio from
 * `dist/gui`; the page talks to this process over the WebView IPC channel and
 * every message maps onto the same N-API functions the CLI uses.
 */

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { NotImplementedError } from './errors.js';
import { nativeBinding } from './native.js';

const here = path.dirname(fileURLToPath(import.meta.url));

/** Lifecycle of one GUI host. */
export interface GuiStatus {
    readonly state: 'open' | 'closed' | 'not_open' | 'failed';
    readonly code: string | null;
    readonly backend: string | null;
    readonly platform: string;
    readonly uptimeMs: number;
    readonly message: string | null;
}

/** Result of asking for a window; failures keep the host detail. */
export interface GuiOpenResult {
    readonly ok: boolean;
    readonly status: GuiStatus;
    readonly stage: string | null;
    readonly hresult: number | null;
    readonly attempted: readonly string[];
}

/** Result of closing a window; `closed` is false when nothing was open. */
export interface GuiCloseResult {
    readonly closed: boolean;
    readonly status: GuiStatus;
}

/** Arguments of {@link openGui}. */
export interface OpenGuiOptions {
    readonly title?: string;
    readonly contentDir?: string;
    readonly width?: number;
    readonly height?: number;
}

/** Terminal host status that is not a clean page close. */
export class GuiError extends Error {
    readonly status: GuiStatus;

    constructor(status: GuiStatus) {
        super(status.message ?? `OpenWord window ended with ${status.code ?? status.state}`);
        this.name = 'GuiError';
        this.status = status;
    }
}

/** The GUI is staged next to the compiled CLI as `dist/gui`. */
export function defaultGuiContentDir(): string {
    const dir = path.resolve(here, 'gui');
    if (!fs.existsSync(path.join(dir, 'index.html'))) {
        throw new Error(`GUI folder missing at ${dir}; run the UI build before opening the window`);
    }
    return dir;
}

function native(feature: string) {
    const binding = nativeBinding();
    if (!binding) {
        throw new NotImplementedError(feature);
    }
    return binding;
}

/** Opens the window, or focuses the live one. */
export function openGui(options: OpenGuiOptions = {}): GuiOpenResult {
    const binding = native('gui');
    const raw = binding.openGui({
        title: options.title ?? 'OpenWord',
        contentDir: options.contentDir ?? defaultGuiContentDir(),
        width: options.width,
        height: options.height,
    });
    return JSON.parse(raw) as GuiOpenResult;
}

/** Pumps the host loop once and reports the lifecycle state. */
export function pumpEvents(): GuiStatus {
    return JSON.parse(native('gui').pumpEvents()) as GuiStatus;
}

/** Closes the window; distinguishes "already closed" from a failure. */
export function closeGui(): GuiCloseResult {
    return JSON.parse(native('gui').closeGui()) as GuiCloseResult;
}

/** Reports whether the window is alive. */
export function isGuiOpen(): boolean {
    return native('gui').isGuiOpen();
}

/** Opens the window and pumps until it closes. */
export function runGui(options: OpenGuiOptions = {}, intervalMs = 16): Promise<GuiStatus> {
    const opened = openGui(options);
    if (!opened.ok) {
        return Promise.reject(new GuiError(opened.status));
    }
    return new Promise((resolve, reject) => {
        const timer = setInterval(() => {
            let status: GuiStatus;
            try {
                status = pumpEvents();
            } catch (error) {
                clearInterval(timer);
                reject(error);
                return;
            }
            if (status.state === 'open') {
                return;
            }
            clearInterval(timer);
            if (status.code === 'gui_closed') {
                resolve(status);
            } else {
                reject(new GuiError(status));
            }
        }, intervalMs);
    });
}
