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
 * type and template semantics; `.html`, `.md`, `.txt` and `.rtf` are lossy text
 * projections. RTF export is `partial`: tables degrade to tab-separated rows
 * and pictures and link destinations are dropped with `export.rtf.*` losses.
 */
export type ExportFormat = 'docx' | 'dotx' | 'pdf' | 'html' | 'md' | 'txt' | 'rtf';

/** What the installed engine can do, per format and per capability. */
export interface FormatCapability {
    readonly import: SupportLevel;
    readonly edit: SupportLevel;
    readonly export: SupportLevel;
    /** Loss codes a `partial` route can report, such as `import.rtf.table`. */
    readonly lossCodes?: readonly string[];
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
    /** Document file to start from; its blocks become the new document. */
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

/** Counts of one document at its committed revision. */
export interface StatsReport {
    readonly revision: number;
    /** Every reachable block node, in document order. */
    readonly blocks: number;
    /** Paragraph blocks, including table cell paragraphs. */
    readonly paragraphs: number;
    /** Heading count per section level, such as `{ "1": 2 }`. */
    readonly headings: Readonly<Record<string, number>>;
    readonly tables: number;
    /** Rows across every table's header, body and footer bands. */
    readonly tableRows: number;
    /** Whitespace-separated words in titles, paragraphs, list items, code blocks and cells. */
    readonly words: number;
    readonly characters: number;
    readonly charactersWithoutWhitespace: number;
    /** `words / 200`, rounded up. */
    readonly readingMinutes: number;
}

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
    | {
          readonly type: 'list.append';
          readonly ordered?: boolean;
          readonly items: readonly string[];
          /** First number of an ordered list. Defaults to 1; invalid for unordered lists. */
          readonly start?: number;
      }
    | { readonly type: 'quote.append'; readonly text: string }
    | { readonly type: 'code.append'; readonly text: string; readonly language?: string }
    | { readonly type: 'table.append'; readonly rows: readonly (readonly string[])[] }
    | { readonly type: 'metadata.set'; readonly title?: string; readonly author?: string }
    | { readonly type: 'document.replaceText'; readonly find: string; readonly replace: string; readonly limit?: number }
    | { readonly type: 'block.remove'; readonly id: string }
    | { readonly type: 'paragraph.insert'; readonly index: number; readonly text: string }
    | { readonly type: 'table.setCell'; readonly table: number; readonly row: number; readonly column: number; readonly text: string }
    | { readonly type: 'table.insertRow'; readonly table: number; readonly row: number; readonly cells?: readonly string[] }
    | { readonly type: 'table.deleteRow'; readonly table: number; readonly row: number }
    | { readonly type: 'table.insertColumn'; readonly table: number; readonly column: number; readonly cells?: readonly string[] }
    | { readonly type: 'table.deleteColumn'; readonly table: number; readonly column: number }
    | { readonly type: 'paragraph.update'; readonly id?: string; readonly index?: number; readonly text: string }
    | { readonly type: 'block.move'; readonly id: string; readonly index: number }
    | {
          readonly type: 'template.fill';
          readonly values: Readonly<Record<string, string | number | boolean>>;
      };

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
    /** Address of the block an operation inserted or updated. */
    readonly id?: string;
    /** List items created by `list.append`. */
    readonly items?: number;
    /** Whether the list appended by `list.append` is ordered. */
    readonly ordered?: boolean;
    /** First number requested by `list.append`; `null` when none was given or the list is unordered. */
    readonly start?: number | null;
    /** Language tag of the code block appended by `code.append`; `null` when none was given. */
    readonly language?: string | null;
    /** Table index addressed by a table operation. */
    readonly table?: number;
    /** Row index addressed by a row operation. */
    readonly row?: number;
    /** Grid column index addressed by a column operation. */
    readonly column?: number;
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

/** Options of `OpenWord.replace()`. */
export interface ReplaceOptions {
    /** Match case exactly. Defaults to `true`. */
    readonly caseSensitive?: boolean;
    /** Replace only matches with no word character on either side. Defaults to `false`. */
    readonly wholeWord?: boolean;
    /** Maximum number of replacements across the whole document. */
    readonly limit?: number;
    /** Reject the pass when the document is not at this revision. */
    readonly expectedRevision?: number;
}

/** Request of `OpenWord.replace()`. */
export interface ReplaceRequest extends ReplaceOptions {
    /** Text to find. Must be non-empty. */
    readonly find: string;
    /** Replacement text. An empty string deletes every match. */
    readonly replace: string;
}

/** One block a replacement pass changed. */
export interface ReplaceAffectedBlock {
    /** Stable block address: `section-N` for sections, `node-N` otherwise. */
    readonly blockId: string;
    /** Depth-first position of the block in document order. */
    readonly blockIndex: number;
    /** Occurrences replaced inside this block. */
    readonly replaced: number;
}

/** Result of one replacement pass. A pass without matches changes nothing. */
export interface ReplaceReport {
    /** Committed summary after the pass. */
    readonly document: DocumentSummary;
    /** Revision after the pass. */
    readonly revision: number;
    /** Total occurrences replaced. */
    readonly replaced: number;
    /** Blocks that changed, in document order. */
    readonly affected: readonly ReplaceAffectedBlock[];
}

/** One thing an import could not keep. */
export interface ImportLoss {
    readonly code: string;
    readonly severity: 'warning' | 'error';
    readonly message: string;
}

/** Import completeness of one opened document. */
export interface ImportReport {
    /** False when the importer recorded at least one loss. */
    readonly complete: boolean;
    readonly losses: readonly ImportLoss[];
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

/**
 * Result of `OpenWord.undo()`.
 *
 * History is process-local: it lives on the open handle and is never written to
 * a `.oword` container.
 */
export interface UndoResult {
    readonly document: DocumentSummary;
    readonly revision: number;
    /** Number of mutations restored. */
    readonly undone: number;
    /** Whether at least one undone mutation can be replayed. */
    readonly redoAvailable: boolean;
}

/** Result of `OpenWord.redo()`. */
export interface RedoResult {
    readonly document: DocumentSummary;
    readonly revision: number;
    /** Number of undone mutations replayed. */
    readonly redone: number;
    /** Whether at least one mutation can still be undone. */
    readonly undoAvailable: boolean;
}

/** Options of `OpenWord.merge()`. */
export interface MergeOptions {
    /** Only `append` is supported today. */
    readonly mode?: 'append';
}

/**
 * Result of `OpenWord.merge()`.
 *
 * The donor document is opened through the same routing as `open()`; its blocks
 * are deep-copied to the end of the receiver with fresh node ids.
 */
export interface MergeResult {
    /** Number of block nodes appended, including nested children. */
    readonly blocksAdded: number;
    /** Committed summary after the merge. */
    readonly summary: DocumentSummary;
    readonly revision: number;
    /** Import losses carried by the merged document, plus the receiver's own. */
    readonly losses: readonly ImportLoss[];
}
