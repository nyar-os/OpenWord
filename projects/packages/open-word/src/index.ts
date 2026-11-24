/**
 * Authoring surface of `@wae/open-word`.
 *
 * Everything here is a declared contract: the session and format registry exist,
 * while every capability rejects with `capability/not-implemented` until the
 * shared engine lands. Document semantics, styles, templates, rendering and DOCX
 * exchange belong upstream and are never re-implemented in this package.
 */

export { OpenWord } from './open-word.js';
export { GuiError, closeGui, defaultGuiContentDir, isGuiOpen, openGui, pumpEvents, runGui } from './gui.js';
export type { GuiCloseResult, GuiOpenResult, GuiStatus, OpenGuiOptions } from './gui.js';
export { NotImplementedError, OpenWordError, errorCodes, isOpenWordError, renderErrorCopy, setErrorLocale } from './errors.js';
export type { OpenWordErrorCode, OpenWordErrorDetails, OpenWordErrorOptions, OpenWordErrorParams } from './errors.js';
export { defaultIntentFor, documentFormats, formatFromPath, formatInfo, formatRegistry } from './formats.js';
export type {
    DocumentFormat,
    DocumentFormatInfo,
    DocumentIntent,
    FormatFamily,
    FormatMarkup,
    FormatOpenKind,
} from './formats.js';
export type * from './types.js';
