/**
 * `new OpenWord(options?)` authoring session.
 *
 * The constructor is synchronous and opens nothing: no network, no disk scan,
 * no window, no file. Native initialisation and imports happen inside the async
 * methods, so a failure is always awaited and always carries a stable code.
 */

import { OpenWordError } from './errors.js';
import * as service from './service.js';
import type {
    Document,
    DocumentCapabilities,
    DocumentCreateInput,
    DocumentOpenInput,
    EditOperation,
    EditOptions,
    ExportFormat,
    ExportReport,
    FindHit,
    FindOptions,
    InspectOptions,
    InspectReport,
    MergeOptions,
    MergeResult,
    OpenWordOptions,
    RedoResult,
    SaveOptions,
    SaveReport,
    UndoResult,
} from './types.js';

/** Authoring session. One instance manages one declared document surface. */
export class OpenWord {
    /** BCP-47 tag used for diagnostics and default copy. */
    readonly locale: string;
    private closed = false;
    private active: number | null = null;
    private readonly handles = new Set<number>();

    constructor(options: OpenWordOptions = {}) {
        this.locale = options.locale ?? 'en';
    }

    /** Whether {@link dispose} has run. */
    get disposed(): boolean {
        return this.closed;
    }

    /**
     * Format, checker, renderer and export disclosure for the installed engine.
     * Read this instead of treating a method name as a capability.
     */
    async capabilities(): Promise<DocumentCapabilities> {
        this.assertActive();
        return service.capabilities();
    }

    /** Creates a document from a title and an optional template. */
    async create(options: DocumentCreateInput = {}): Promise<Document> {
        this.assertActive();
        assertOptionalString(options.title, 'title');
        assertOptionalString(options.template, 'template');
        assertOptionalString(options.author, 'author');
        const created = service.create({ title: options.title, template: options.template, author: options.author });
        this.handles.add(created.handle);
        this.active = created.handle;
        return created.document;
    }

    /** Opens a native project or a document at a path. */
    async open(input: DocumentOpenInput): Promise<Document> {
        this.assertActive();
        const path = input?.path;
        assertPath(path);
        const intent = input?.intent ?? 'edit';
        if (intent !== 'edit') {
            throw new OpenWordError(
                'format/unsupported',
                `native documents open for editing; \`${intent}\` needs the projection pipeline`,
                { details: { feature: `open.intent.${intent}` } },
            );
        }
        const opened = service.open(path);
        this.handles.add(opened.handle);
        this.active = opened.handle;
        return opened.document;
    }

    /** Writes the active document to a native project path. */
    async save(target: string, options: SaveOptions = {}): Promise<SaveReport> {
        this.assertActive();
        assertPath(target);
        return service.save(this.requireHandle(), target, options.overwrite === true);
    }

    /** Exports the active document to one projection target. */
    async export(options: { readonly format: ExportFormat; readonly target: string; readonly overwrite?: boolean }): Promise<ExportReport> {
        this.assertActive();
        const format = options?.format;
        if (typeof format !== 'string' || format.length === 0) {
            throw new OpenWordError('session/argument-invalid', '`format` must be a non-empty string');
        }
        assertPath(options?.target);
        return service.exportDocument(this.requireHandle(), format, options.target, options.overwrite !== false);
    }

    /**
     * Applies one atomic edit batch and returns the new committed summary.
     *
     * `expectedRevision` rejects a batch written against an older revision with
     * `document/revision-conflict` before anything mutates.
     */
    async edit(operations: readonly EditOperation[], options: EditOptions = {}): Promise<Document> {
        this.assertActive();
        assertOperations(operations);
        const expectedRevision = options?.expectedRevision;
        if (expectedRevision !== undefined && (!Number.isInteger(expectedRevision) || expectedRevision < 0)) {
            throw new OpenWordError('session/argument-invalid', '`expectedRevision` must be a non-negative integer');
        }
        const report = service.edit(this.requireHandle(), { operations, expectedRevision });
        return report.document;
    }

    /**
     * Undoes the last `steps` committed mutations (default 1).
     *
     * History is process-local and capped at 50 mutations; asking for more steps
     * than are retained rejects with `session/argument-invalid` and changes
     * nothing. A successful undo bumps the revision once.
     */
    async undo(options: { readonly steps?: number } = {}): Promise<UndoResult> {
        this.assertActive();
        const steps = options?.steps;
        assertSteps(steps);
        return service.undo(this.requireHandle(), steps);
    }

    /** Replays undone mutations, most recent first. */
    async redo(options: { readonly steps?: number } = {}): Promise<RedoResult> {
        this.assertActive();
        const steps = options?.steps;
        assertSteps(steps);
        return service.redo(this.requireHandle(), steps);
    }

    /**
     * Appends another document's blocks to the active document.
     *
     * The donor is opened through the same routing as `open()`, so `.docx`,
     * `.md`, `.html`, `.rtf`, `.txt` and `.oword` are accepted; merging the
     * file the document was opened from is allowed.
     */
    async merge(path: string, options: MergeOptions = {}): Promise<MergeResult> {
        this.assertActive();
        assertPath(path);
        const mode: string = options?.mode ?? 'append';
        if (mode !== 'append') {
            throw new OpenWordError('object/unsupported', `merge mode \`${mode}\` is not supported`);
        }
        return service.merge(this.requireHandle(), path, mode);
    }

    /** Inspects an input through a transient handle that is always released. */
    async inspect(input: DocumentOpenInput, options?: InspectOptions): Promise<InspectReport> {
        this.assertActive();
        const path = input?.path;
        assertPath(path);
        const opened = service.open(path);
        try {
            return service.inspect(opened.handle, options?.checks);
        } finally {
            closeQuietly(opened.handle);
        }
    }

    /**
     * Finds text across section titles, paragraphs, list items and table cells.
     *
     * Offsets are character positions inside the matched block's plain text.
     */
    async find(query: string, options: FindOptions = {}): Promise<readonly FindHit[]> {
        this.assertActive();
        if (typeof query !== 'string' || query.length === 0) {
            throw new OpenWordError('session/argument-invalid', '`query` must be a non-empty string');
        }
        const limit = options?.limit;
        if (limit !== undefined && (!Number.isInteger(limit) || limit < 1)) {
            throw new OpenWordError('session/argument-invalid', '`limit` must be a positive integer');
        }
        return service.find(this.requireHandle(), query, limit);
    }

    /** Releases every tracked handle. Idempotent. */
    async dispose(): Promise<void> {
        for (const handle of this.handles) {
            closeQuietly(handle);
        }
        this.handles.clear();
        this.active = null;
        this.closed = true;
    }

    /** @internal Throws `document/not-found` when no document is active. */
    private requireHandle(): number {
        if (this.active === null) {
            throw new OpenWordError('document/not-found', 'no document is open in this session; call `create` or `open` first');
        }
        return this.active;
    }

    /** @internal Throws `session/disposed` once the session is gone. */
    assertActive(): void {
        if (this.closed) {
            throw new OpenWordError('session/disposed', 'the OpenWord session has been disposed; create a new instance');
        }
    }
}

function assertPath(value: unknown): asserts value is string {
    if (typeof value !== 'string' || value.length === 0) {
        throw new OpenWordError('session/argument-invalid', '`path` must be a non-empty string');
    }
}

function assertOptionalString(value: unknown, field: string): void {
    if (value !== undefined && typeof value !== 'string') {
        throw new OpenWordError('session/argument-invalid', `\`${field}\` must be a string when present`);
    }
}

function assertSteps(value: unknown): void {
    if (value !== undefined && (!Number.isInteger(value) || (value as number) < 1)) {
        throw new OpenWordError('session/argument-invalid', '`steps` must be a positive integer');
    }
}

function assertOperations(value: unknown): asserts value is readonly EditOperation[] {
    if (!Array.isArray(value) || value.length === 0) {
        throw new OpenWordError('session/argument-invalid', '`operations` must be a non-empty array');
    }
    for (const operation of value) {
        if (typeof operation !== 'object' || operation === null || typeof (operation as { type?: unknown }).type !== 'string') {
            throw new OpenWordError('session/argument-invalid', 'each edit operation needs a `type` string');
        }
    }
}

function closeQuietly(handle: number): void {
    try {
        service.close(handle);
    } catch {
        // A handle that never reached the engine is already released; disposal
        // must not mask the original failure.
    }
}
