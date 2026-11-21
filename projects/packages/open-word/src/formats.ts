/**
 * Document format family.
 *
 * Word extensions encode two things at once: the container (`x` = OOXML,
 * `m` = macro-enabled, none = 97-2003 binary) and the intended use (`dot*` =
 * template). Open formats such as ODT, RTF, plain text, Markdown and HTML are
 * grouped separately, and PDF is a fixed projection that never becomes an
 * editable document.
 *
 * Classification is **not** a support claim. What the installed engine can read,
 * write or render is reported by `OpenWord.capabilities()`; an extension listed
 * here may still be `unsupported`.
 */

/** Intent applied when the caller does not override it. */
export type DocumentIntent = 'edit' | 'template' | 'read';

/** Grouping used for capability reporting and user-facing wording. */
export type FormatFamily = 'core' | 'template' | 'legacy' | 'interchange' | 'plain' | 'markup' | 'fixed';

/** Container encoding behind an extension. */
export type FormatMarkup = 'ooxml' | 'binary' | 'odf' | 'text' | 'markdown' | 'html' | 'fixed';

/** What an extension can be consumed as. `none` means it never becomes a document. */
export type FormatOpenKind = 'document' | 'template' | 'none';

/** Every extension the product must classify. */
export const documentFormats = [
    'docx',
    'docm',
    'dotx',
    'dotm',
    'doc',
    'rtf',
    'odt',
    'txt',
    'md',
    'html',
    'pdf',
] as const;

/** Union of {@link documentFormats}. */
export type DocumentFormat = (typeof documentFormats)[number];

/** What a format is, independent of whether it is implemented. */
export interface DocumentFormatInfo {
    readonly format: DocumentFormat;
    readonly family: FormatFamily;
    readonly markup: FormatMarkup;
    /** Whether the container may hold VBA macros. Such parts are never executed. */
    readonly macroEnabled: boolean;
    readonly opensAs: FormatOpenKind;
    /** Intent applied to `open()` when the extension arrives without one. */
    readonly defaultIntent: DocumentIntent | null;
}

type FormatRegistryEntry = Omit<DocumentFormatInfo, 'format'>;

/** Extension to classification. Values describe the format, never a capability. */
export const formatRegistry: Readonly<Record<DocumentFormat, FormatRegistryEntry>> = Object.freeze({
    docx: { family: 'core', markup: 'ooxml', macroEnabled: false, opensAs: 'document', defaultIntent: 'edit' },
    docm: { family: 'core', markup: 'ooxml', macroEnabled: true, opensAs: 'document', defaultIntent: 'edit' },
    dotx: { family: 'template', markup: 'ooxml', macroEnabled: false, opensAs: 'template', defaultIntent: 'template' },
    dotm: { family: 'template', markup: 'ooxml', macroEnabled: true, opensAs: 'template', defaultIntent: 'template' },
    doc: { family: 'legacy', markup: 'binary', macroEnabled: false, opensAs: 'none', defaultIntent: null },
    rtf: { family: 'interchange', markup: 'text', macroEnabled: false, opensAs: 'document', defaultIntent: 'read' },
    odt: { family: 'interchange', markup: 'odf', macroEnabled: false, opensAs: 'document', defaultIntent: 'edit' },
    txt: { family: 'plain', markup: 'text', macroEnabled: false, opensAs: 'document', defaultIntent: 'read' },
    md: { family: 'markup', markup: 'markdown', macroEnabled: false, opensAs: 'document', defaultIntent: 'read' },
    html: { family: 'markup', markup: 'html', macroEnabled: false, opensAs: 'document', defaultIntent: 'read' },
    pdf: { family: 'fixed', markup: 'fixed', macroEnabled: false, opensAs: 'none', defaultIntent: null },
});

/** Returns the full classification of one format. */
export function formatInfo(format: DocumentFormat): DocumentFormatInfo {
    return { format, ...formatRegistry[format] };
}

/** Extracts a classified extension from a path, ignoring case. */
export function formatFromPath(path: string): DocumentFormat | undefined {
    const extension = path.slice(path.lastIndexOf('.') + 1).toLowerCase();
    return (documentFormats as readonly string[]).includes(extension) ? (extension as DocumentFormat) : undefined;
}

/** Default open intent of a format, or `null` when it never opens as a document. */
export function defaultIntentFor(format: DocumentFormat): DocumentIntent | null {
    return formatRegistry[format].defaultIntent;
}
