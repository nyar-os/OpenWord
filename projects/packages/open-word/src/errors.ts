/**
 * Stable error surface of `@wae/open-word`.
 *
 * Codes are contract, not prose: callers compare them, localize around them and
 * never parse the message. A failure therefore keeps its code even when the
 * text is translated.
 */

/** Every stable code the authoring surface may report. */
export const errorCodes = [
    'session/disposed',
    'session/argument-invalid',
    'session/budget-exceeded',
    'document/not-found',
    'document/revision-conflict',
    'object/not-found',
    'object/unsupported',
    'format/unsupported',
    'format/invalid',
    'io/read',
    'io/write',
    'io/exists',
    'io/failure',
    'capability/not-implemented',
    'binding/failure',
] as const;

/** Union of {@link errorCodes}. */
export type OpenWordErrorCode = (typeof errorCodes)[number];

/** Optional locators a failure can carry so callers can point at the object. */
export interface OpenWordErrorDetails {
    readonly revision?: number;
    readonly documentId?: string;
    readonly objectId?: string;
    /** Document format involved in the failure, without the leading dot. */
    readonly format?: string;
    /** Authoring capability involved in the failure, e.g. `document.export`. */
    readonly feature?: string;
}

/** Machine-readable per-code parameters delivered by the binding contract. */
export type OpenWordErrorParams = Readonly<Record<string, unknown>>;

/** Options accepted by {@link OpenWordError}. */
export interface OpenWordErrorOptions {
    readonly recoverable?: boolean;
    readonly details?: OpenWordErrorDetails;
    readonly params?: OpenWordErrorParams;
    readonly cause?: unknown;
}

/** Every failure thrown or rejected by this package. */
export class OpenWordError extends Error {
    /** Stable, comparable code. */
    readonly code: OpenWordErrorCode;
    /** Whether retrying with different input or state can succeed. */
    readonly recoverable: boolean;
    /** Optional locators (`revision`, `documentId`, `objectId`, `format`, `feature`). */
    readonly details: OpenWordErrorDetails;
    /** Typed parameters of the code, e.g. `{ kind, id }` for `object/not-found`. */
    readonly params: OpenWordErrorParams;

    constructor(code: OpenWordErrorCode, message: string, options: OpenWordErrorOptions = {}) {
        super(message, options.cause === undefined ? undefined : { cause: options.cause });
        this.name = 'OpenWordError';
        this.code = code;
        this.recoverable = options.recoverable ?? false;
        this.details = Object.freeze({ ...options.details });
        this.params = Object.freeze({ ...options.params });
    }
}

/**
 * Reported by declared methods whose backing capability is not wired yet.
 *
 * The scaffold uses one code for every unimplemented entry point instead of
 * inventing per-method placeholders; each method drops this rejection as it is
 * implemented, and the code disappears with the last one.
 */
export class NotImplementedError extends OpenWordError {
    constructor(feature: string, message = `\`${feature}\` is not implemented yet`) {
        super('capability/not-implemented', message, { recoverable: false, details: { feature } });
        this.name = 'NotImplementedError';
    }
}

/** Narrows an unknown thrown value to {@link OpenWordError}. */
export function isOpenWordError(value: unknown): value is OpenWordError {
    return value instanceof OpenWordError;
}

/**
 * Product copy for each stable code, rendered from its typed `params`.
 *
 * The binding's own text is only a fallback for codes this catalog does not
 * carry yet or whose params are incomplete; the machine-readable `code` and
 * `params` never depend on locale.
 */
const messageCatalog: Readonly<Record<string, Readonly<Record<string, string>>>> = {
    en: {
        'session/disposed': 'the OpenWord session has been disposed; create a new instance',
        'session/argument-invalid': 'the request was rejected: {reason}',
        'session/budget-exceeded': 'the operation exceeded the session budget',
        'document/not-found': 'no live document has handle {handle}',
        'document/revision-conflict': 'the document changed at revision {actual}; the edit expected {expected}',
        'object/not-found': 'the `{kind}` `{id}` was not found',
        'object/unsupported': 'the `{operation}` `{name}` is not supported by this build',
        'format/unsupported': '`{format}` has no route in this build',
        'format/invalid': 'the `{format}` input is invalid: {reason}',
        'io/read': 'could not read `{path}` ({kind})',
        'io/write': 'could not publish `{path}`',
        'io/exists': '`{path}` already exists; pass overwrite to replace it',
        'io/failure': 'the file operation failed ({kind})',
        'capability/not-implemented': 'the `{feature}` capability is not implemented yet',
        'binding/failure': 'the native binding reported a failure without a stable code',
    },
    'zh-CN': {
        'session/disposed': 'OpenWord 会话已释放；请创建新实例',
        'session/argument-invalid': '请求被拒绝：{reason}',
        'session/budget-exceeded': '操作超出会话预算',
        'document/not-found': '句柄 {handle} 没有活动文档',
        'document/revision-conflict': '文档已变更到修订 {actual}；本次编辑期望 {expected}',
        'object/not-found': '未找到 `{kind}` `{id}`',
        'object/unsupported': '此构建不支持 `{operation}` `{name}`',
        'format/unsupported': '`{format}` 在此构建中没有路由',
        'format/invalid': '`{format}` 输入无效：{reason}',
        'io/read': '无法读取 `{path}`（{kind}）',
        'io/write': '无法发布 `{path}`',
        'io/exists': '`{path}` 已存在；传入 overwrite 以替换',
        'io/failure': '文件操作失败（{kind}）',
        'capability/not-implemented': '`{feature}` 能力尚未实现',
        'binding/failure': '原生绑定返回了没有稳定码的失败',
    },
};

let currentLocale = 'en';

/** Selects the locale used to render error copy; `zh*` maps to `zh-CN`. */
export function setErrorLocale(tag: string): void {
    currentLocale = tag.toLowerCase().startsWith('zh') ? 'zh-CN' : 'en';
}

/**
 * Renders the product copy for a code and its params.
 *
 * Unknown codes, missing params and missing translations fall back to the
 * binding's own text, so a replaced or removed native message never changes
 * which branch a caller takes: that branch is decided by `code` and `params`.
 */
export function renderErrorCopy(code: string, params: OpenWordErrorParams, fallback: string): string {
    const template = messageCatalog[currentLocale]?.[code];
    if (template === undefined) {
        return fallback;
    }
    let complete = true;
    const rendered = template.replace(/\{(\w+)\}/g, (match, key: string) => {
        if (params[key] === undefined) {
            complete = false;
            return match;
        }
        return String(params[key]);
    });
    return complete ? rendered : fallback;
}
