/**
 * Orthogonal native operations.
 *
 * One function performs one operation and returns data; argument parsing, help
 * text and exit codes belong to `cli.ts` and to the authoring surface. Nothing
 * here knows about a command line.
 */

import { NotImplementedError, OpenWordError, errorCodes, type OpenWordErrorCode } from './errors.js';
import { nativeBinding } from './native.js';
import type {
    Document,
    DocumentCapabilities,
    DocumentCreateInput,
    DocumentSummary,
    EditReport,
    EditRequest,
    ExportFormat,
    ExportReport,
    InspectCheck,
    InspectReport,
    SaveReport,
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
    const message = error instanceof Error ? error.message : String(error);
    const separator = message.indexOf(': ');
    const code = separator > 0 ? message.slice(0, separator) : '';
    if (knownCodes.has(code)) {
        return new OpenWordError(code as OpenWordErrorCode, message.slice(separator + 2), { details: { feature: code } });
    }
    return new OpenWordError('format/invalid', message, { cause: error });
}

/** A released-on-close document handle plus its committed summary. */
export interface DocumentHandle {
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

/** Applies one edit batch and returns the new committed summary. */
export function edit(handle: number, request: EditRequest): EditReport {
    return decode<EditReport>(() => binding('edit').document_edit(handle, JSON.stringify(request)));
}

/** Writes one export target from a document handle. */
export function exportDocument(handle: number, format: ExportFormat, target: string, overwrite = true): ExportReport {
    return decode<ExportReport>(() => binding('export').document_export(handle, format, target, overwrite));
}

/** Releases one document handle. */
export function close(handle: number): void {
    binding('document.close').document_close(handle);
}
