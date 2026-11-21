/**
 * Authoring types of `@wae/open-word`.
 *
 * This is the declared shape of the authoring surface: `new OpenWord(options?)`
 * creates a session, `create`/`open` return an opened `Document`, and every
 * summary and report is addressed by a stable identity plus a revision.
 *
 * Geometry and layout in the authoring surface are points. The engine stores
 * twips internally; screen pixels never enter the model.
 */

import type { DocumentFormat, DocumentIntent } from './formats.js';

/** Hard limits applied before work starts. */
export interface ResourceBudgets {
    readonly maxDocumentBytes?: number;
    readonly maxAssetBytes?: number;
    readonly maxParagraphs?: number;
}

/** Structured diagnostic emitted during import, inspection or export. */
export interface DocumentDiagnostic {
    readonly code: string;
    readonly severity: 'info' | 'warning' | 'error';
    readonly documentId?: string;
    readonly objectId?: string;
    readonly recoverable: boolean;
    readonly message: string;
}

/** Session options. Constructing a session opens no window, file or socket. */
export interface OpenWordOptions {
    /** BCP-47 tag used for diagnostics and default copy. Defaults to `en`. */
    readonly locale?: string;
    readonly budgets?: ResourceBudgets;
    readonly diagnostics?: (event: DocumentDiagnostic) => void;
}

/** Disclosure level. `partial` always ships a loss path and diagnostic codes. */
export type SupportLevel = 'supported' | 'partial' | 'unsupported';

/** Inspection rules understood by `inspect()`. */
export type InspectCheck = 'styles' | 'fonts' | 'images' | 'sections' | 'tables' | 'links';

/** Per-document render targets. */
export type RenderFormat = 'svg' | 'png' | 'pdf';

/**
 * Targets `export()` can write.
 *
 * `.docx` and `.dotx` share one OOXML package writer and differ only in content
 * type and template semantics; `.html`, `.md` and `.txt` are lossy text
 * projections.
 */
export type ExportFormat = 'docx' | 'dotx' | 'pdf' | 'html' | 'md' | 'txt';

/** What the installed engine can do, per format and per capability. */
export interface FormatCapability {
    readonly import: SupportLevel;
    readonly edit: SupportLevel;
    readonly export: SupportLevel;
}

/** Result of `OpenWord.capabilities()`. Read this instead of assuming a method works. */
export interface DocumentCapabilities {
    readonly engine: { readonly package: string; readonly version: string };
    /** The native project container; `unsupported` means nothing can be saved yet. */
    readonly project: SupportLevel;
    readonly formats: Readonly<Record<DocumentFormat, FormatCapability>>;
    readonly checks: Readonly<Record<InspectCheck, SupportLevel>>;
    readonly renderers: Readonly<Record<RenderFormat, SupportLevel>>;
    readonly exportTargets: Readonly<Record<ExportFormat, SupportLevel>>;
}

/** Options of `OpenWord.create()`. */
export interface DocumentCreateInput {
    readonly title?: string;
    /** Template file to start from, once template rendering lands. */
    readonly template?: string;
    readonly author?: string;
}

/**
 * Input of `OpenWord.open()`.
 *
 * A path is mandatory, so the engine never guesses whether a string is a file
 * name or serialized content. `intent` defaults to `edit`.
 */
export interface DocumentOpenInput {
    readonly path: string;
    readonly intent?: DocumentIntent;
}

/** One section (page block) of a document summary. */
export interface DocumentSummarySection {
    readonly index: number;
    readonly id: string;
    readonly title: string | null;
}

/** One paragraph of a document summary. */
export interface DocumentSummaryParagraph {
    readonly index: number;
    readonly sectionIndex: number;
    /** Resolved paragraph style name, or `null` when the engine has none. */
    readonly style: string | null;
    readonly text: string;
}

/** One table of a document summary. */
export interface DocumentSummaryTable {
    readonly index: number;
    readonly sectionIndex: number;
    readonly rows: number;
    readonly columns: number;
}

/** Summary of one document at its committed revision. */
export interface DocumentSummary {
    readonly id: string;
    readonly revision: number;
    readonly title: string | null;
    readonly sections: readonly DocumentSummarySection[];
    readonly paragraphs: readonly DocumentSummaryParagraph[];
    readonly tables: readonly DocumentSummaryTable[];
}

/** An opened document at its committed revision. */
export type Document = DocumentSummary;

/** Options of `OpenWord.inspect()`. */
export interface InspectOptions {
    readonly checks?: readonly InspectCheck[];
}

/** One finding of an inspection run. */
export interface InspectIssue {
    readonly code: string;
    readonly severity: 'info' | 'warning' | 'error';
    readonly revision: number;
    readonly sectionId?: string;
    readonly objectId?: string;
    readonly recoverable: boolean;
    readonly message: string;
}

/** Result of `inspect()`. Structure and rules only, never an aesthetic score. */
export interface InspectReport {
    readonly revision: number;
    readonly checkerVersion: string;
    readonly passed: boolean;
    readonly issues: readonly InspectIssue[];
}

/** Result of writing a native project. */
export interface SaveReport {
    readonly path: string;
    readonly revision: number;
    readonly bytes: number;
}

/** Options of `OpenWord.save()`. */
export interface SaveOptions {
    /** Replace the target file when it already exists. Defaults to `false`. */
    readonly overwrite?: boolean;
}

/**
 * One atomic edit. Operations are validated as a whole before the first
 * mutation, so a rejected batch never leaves a half-applied revision.
 */
export type EditOperation =
    | { readonly type: 'paragraph.append'; readonly text: string }
    | { readonly type: 'heading.append'; readonly level: number; readonly text: string }
    | { readonly type: 'table.append'; readonly rows: readonly (readonly string[])[] }
    | { readonly type: 'metadata.set'; readonly title?: string; readonly author?: string }
    | { readonly type: 'document.replaceText'; readonly find: string; readonly replace: string; readonly limit?: number }
    | { readonly type: 'block.remove'; readonly id: string }
    | { readonly type: 'paragraph.insert'; readonly index: number; readonly text: string }
    | { readonly type: 'table.setCell'; readonly table: number; readonly row: number; readonly column: number; readonly text: string };

/** Options of `OpenWord.edit()`. */
export interface EditOptions {
    /** Reject the batch when the document is not at this revision. */
    readonly expectedRevision?: number;
}

/** Batch submitted to the engine. */
export interface EditRequest extends EditOptions {
    readonly operations: readonly EditOperation[];
}

/** Outcome of one applied edit operation. */
export interface EditOperationResult {
    /** Operation that produced this report. */
    readonly type: string;
    /** Replaced occurrences of `document.replaceText`. */
    readonly replaced?: number;
    /** Blocks physically removed by `block.remove`. */
    readonly removed?: number;
    /** Address of the block an operation inserted. */
    readonly id?: string;
}

/** Result of one edit batch. */
export interface EditReport {
    readonly document: DocumentSummary;
    readonly revision: number;
    readonly results: readonly EditOperationResult[];
}

/** Options of `OpenWord.find()`. */
export interface FindOptions {
    /** Maximum number of hits to return. Defaults to 100. */
    readonly limit?: number;
}

/** One text match inside a block's plain text. */
export interface FindHit {
    /** Stable block address: `section-N` for sections, `node-N` otherwise. */
    readonly blockId: string;
    /** Depth-first position of the block in document order. */
    readonly blockIndex: number;
    /** Nearest enclosing section id, or `null` at top level. */
    readonly path: string | null;
    /** Character offset of the match start inside the block text. */
    readonly start: number;
    /** Character offset of the match end inside the block text. */
    readonly end: number;
    /** Matched text. */
    readonly text: string;
}

/** One thing a projection could not keep. */
export interface ExportLoss {
    readonly code: string;
    readonly severity: 'warning' | 'error';
    readonly sectionId?: string;
    readonly objectId?: string;
    readonly message: string;
}

/** Result of `export()`. Reports what the target actually kept. */
export interface ExportReport {
    readonly revision: number;
    readonly format: ExportFormat;
    readonly target: string;
    readonly losses: readonly ExportLoss[];
    /** False for projections such as PDF that cannot be reopened for editing. */
    readonly editable: boolean;
}
