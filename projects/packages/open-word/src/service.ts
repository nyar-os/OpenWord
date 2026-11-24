/**
 * Orthogonal native operations.
 *
 * One function performs one operation and returns data; argument parsing, help
 * text and exit codes belong to `cli.ts` and to the authoring surface. Nothing
 * here knows about a command line.
 */

import {
    NotImplementedError,
    OpenWordError,
    errorCodes,
    renderErrorCopy,
    type OpenWordErrorCode,
    type OpenWordErrorParams,
} from './errors.js';
import { nativeBinding } from './native.js';
import type {
    Document,
    DocumentCapabilities,
    DocumentCreateInput,
    DocumentOutline,
    DocumentSummary,
    EditReport,
    EditRequest,
    ExportFormat,
    ExportReport,
    FindHit,
    ImportReport,
    InspectCheck,
    InspectReport,
    MergeResult,
    RedoResult,
    ReplaceReport,
    ReplaceRequest,
    SaveReport,
    StatsReport,
    UndoResult,
} from './types.js';

const knownCodes = new Set<string>(errorCodes);

function binding(feature: string) {
    const native = nativeBinding();
    if (!native) {
        throw new NotImplementedError(feature);
    }
    return native;
}

function decode<T>(work: () => string): T {
    try {
        return JSON.parse(work()) as T;
    } catch (error) {
        throw translate(error);
    }
}

function translate(error: unknown): OpenWordError {
    if (error instanceof OpenWordError) {
        return error;
    }
    const native = error instanceof Error ? (error as Error & { code?: unknown; detail?: unknown }) : undefined;
    const message = native ? native.message : String(error);
    const rawCode = native?.code;
    const code = typeof rawCode === 'string' ? rawCode : undefined;
    if (code !== undefined) {
        const params = readParams(native?.detail);
        return new OpenWordError(code as OpenWordErrorCode, renderErrorCopy(code, params, message), { params, cause: error });
    }
    // Compatibility entry for addons older than the structured contract, which
    // concatenated `code: message` into the Error text. Current bindings always
    // carry `code` and `detail.params`, so this split never runs against them.
    const separator = message.indexOf(': ');
    const legacy = separator > 0 ? message.slice(0, separator) : '';
    if (knownCodes.has(legacy)) {
        return new OpenWordError(legacy as OpenWordErrorCode, message.slice(separator + 2), { cause: error });
    }
    return new OpenWordError('binding/failure', message, { cause: error });
}

function readParams(detail: unknown): OpenWordErrorParams {
    if (typeof detail === 'object' && detail !== null) {
        const params = (detail as { params?: unknown }).params;
        if (typeof params === 'object' && params !== null && !Array.isArray(params)) {
            return params as OpenWordErrorParams;
        }
    }
    return {};
}

/** A released-on-close document handle plus its committed summary and import losses. */
export interface DocumentHandle extends ImportReport {
    readonly handle: number;
    readonly document: Document;
}

/** Reads the capability matrix of the installed addon. */
export function capabilities(): DocumentCapabilities {
    return decode<DocumentCapabilities>(() => binding('capabilities').capabilities());
}

/** Creates a document and returns its handle and committed summary. */
export function create(request: DocumentCreateInput): DocumentHandle {
    return decode<DocumentHandle>(() =>
        binding('create').document_create(
            JSON.stringify({ title: request.title, template: request.template, author: request.author }),
        ),
    );
}

/** Opens a document at a path and returns its handle and committed summary. */
export function open(path: string): DocumentHandle {
    return decode<DocumentHandle>(() => binding('open').document_open(path));
}

/** Writes one document identified by a handle to a native project path. */
export function save(handle: number, target: string, overwrite: boolean): SaveReport {
    return decode<SaveReport>(() => binding('save').document_save(handle, target, overwrite));
}

/** Runs the declared rule checks against one document handle. */
export function inspect(handle: number, checks?: readonly InspectCheck[]): InspectReport {
    return decode<InspectReport>(() =>
        checks === undefined
            ? binding('inspect').document_inspect(handle)
            : binding('inspect').document_inspect(handle, JSON.stringify(checks)),
    );
}

/** Reads the committed summary of one document handle. */
export function summary(handle: number): DocumentSummary {
    return decode<DocumentSummary>(() => binding('summary').document_summary(handle));
}

/** Reads the nested heading outline of one document handle. */
export function outline(handle: number): DocumentOutline {
    return decode<DocumentOutline>(() => binding('outline').document_outline(handle));
}

/** Reads block, word and reading-time statistics of one document handle. */
export function stats(handle: number): StatsReport {
    return decode<StatsReport>(() => binding('stats').word_stats(handle));
}

/** Searches one document for text and returns the matches. */
export function find(handle: number, query: string, limit?: number): readonly FindHit[] {
    return decode<FindHit[]>(() =>
        limit === undefined
            ? binding('find').document_find(handle, query)
            : binding('find').document_find(handle, query, limit),
    );
}

/** Replaces text in one document and reports the affected blocks. */
export function replace(handle: number, request: ReplaceRequest): ReplaceReport {
    return decode<ReplaceReport>(() => binding('replace').document_replace(handle, JSON.stringify(request)));
}

/** Applies one edit batch and returns the new committed summary. */
export function edit(handle: number, request: EditRequest): EditReport {
    return decode<EditReport>(() => binding('edit').document_edit(handle, JSON.stringify(request)));
}

/** Undoes committed edits on a document handle. */
export function undo(handle: number, steps?: number): UndoResult {
    return decode<UndoResult>(() =>
        steps === undefined ? binding('undo').document_undo(handle) : binding('undo').document_undo(handle, steps),
    );
}

/** Replays undone edits on a document handle. */
export function redo(handle: number, steps?: number): RedoResult {
    return decode<RedoResult>(() =>
        steps === undefined ? binding('redo').document_redo(handle) : binding('redo').document_redo(handle, steps),
    );
}

/** Appends another document's blocks to a document handle. */
export function merge(handle: number, path: string, mode = 'append'): MergeResult {
    return decode<MergeResult>(() => binding('merge').document_merge(handle, path, mode));
}

/** Writes one export target from a document handle. */
export function exportDocument(handle: number, format: ExportFormat, target: string, overwrite = true): ExportReport {
    return decode<ExportReport>(() => binding('export').document_export(handle, format, target, overwrite));
}

/** Releases one document handle. */
export function close(handle: number): void {
    binding('document.close').document_close(handle);
}
