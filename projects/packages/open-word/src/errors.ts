/**
 * Stable error surface of `@wae/open-word`.
 *
 * Codes are contract, not prose: callers compare them, localize around them and
 * never parse the message. A failure therefore keeps its code even when the
 * text is translated.
 */

/** Every stable code the document surface may report. */
export const errorCodes = [
    'session/disposed',
    'session/argument-invalid',
    'session/budget-exceeded',
    'document/not-found',
    'document/revision-conflict',
    'io/read',
    'io/write',
    'io/exists',
    'io/failure',
    'object/not-found',
    'object/unsupported',
    'format/unsupported',
    'format/invalid',
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
    /** Format involved in the failure, without the leading dot. */
    readonly format?: string;
    /** Capability involved in the failure, e.g. `document.export`. */
    readonly feature?: string;
}

/** A payload that carries no parameters of its own. */
type NoParams = { readonly [key: string]: never };

/**
 * Frozen payload of every code.
 *
 * These shapes are contract: the binding validates against them and the product
 * renders copy from them, so a code never carries prose it has to be re-parsed.
 * Only the translation changes between locales, never the payload.
 */
export interface ErrorParamsByCode {
    'session/disposed': NoParams;
    'session/argument-invalid': { readonly reason: string };
    'session/budget-exceeded': NoParams;
    'document/not-found': { readonly handle?: number };
    'document/revision-conflict': { readonly actual: number; readonly expected?: number };
    'io/read': { readonly path: string; readonly kind: string };
    'io/write': { readonly path: string };
    'io/exists': { readonly path: string };
    'io/failure': { readonly reason: string };
    'object/not-found': { readonly kind: string; readonly id: string };
    'object/unsupported': { readonly operation: string; readonly name: string };
    'format/unsupported': { readonly format: string };
    'format/invalid': { readonly format: string; readonly reason: string };
    'capability/not-implemented': { readonly feature: string };
    'binding/failure': { readonly reason: string; readonly code?: string };
}

/** Union of every frozen payload. */
export type OpenWordErrorParams = ErrorParamsByCode[OpenWordErrorCode];

/**
 * One failure as a discriminated union: narrowing on `code` narrows `params`.
 *
 * Callers branch on {@link OpenWordFailure.code} and read the payload without a
 * cast, so no branch depends on the wording of a message.
 */
export type OpenWordFailure = {
    [Code in OpenWordErrorCode]: { readonly code: Code; readonly params: ErrorParamsByCode[Code] };
}[OpenWordErrorCode];

/** Field kinds the validator understands; `?` marks an optional field. */
type FieldKind = 'string' | 'number' | 'string?' | 'number?';

const paramSpecs: Readonly<Record<OpenWordErrorCode, Readonly<Record<string, FieldKind>>>> = {
    'session/disposed': {},
    'session/argument-invalid': { reason: 'string' },
    'session/budget-exceeded': {},
    'document/not-found': { handle: 'number?' },
    'document/revision-conflict': { actual: 'number', expected: 'number?' },
    'io/read': { path: 'string', kind: 'string' },
    'io/write': { path: 'string' },
    'io/exists': { path: 'string' },
    'io/failure': { reason: 'string' },
    'object/not-found': { kind: 'string', id: 'string' },
    'object/unsupported': { operation: 'string', name: 'string' },
    'format/unsupported': { format: 'string' },
    'format/invalid': { format: 'string', reason: 'string' },
    'capability/not-implemented': { feature: 'string' },
    'binding/failure': { reason: 'string', code: 'string?' },
};

/** Why a payload was refused. */
export interface ErrorParamsRejection {
    /** `unknown-code`: the code is outside the contract; `malformed`: the payload does not match its shape. */
    readonly kind: 'unknown-code' | 'malformed';
    /** Machine-readable reason, e.g. `missing key `id``. */
    readonly reason: string;
}

/** Outcome of validating one payload against its code's frozen shape. */
export type ErrorParamsResult =
    | { readonly ok: true; readonly params: OpenWordErrorParams }
    | { readonly ok: false; readonly rejection: ErrorParamsRejection };

/**
 * Validates one payload against its code's frozen shape.
 *
 * A payload must be a plain object carrying exactly the declared keys, each of
 * the declared type. Unknown codes, prose, arrays, missing keys, extra keys and
 * mistyped values are refused so nothing half-formed reaches a branch.
 */
export function parseErrorParams(code: string, payload: unknown): ErrorParamsResult {
    if (!isContractedCode(code)) {
        return { ok: false, rejection: { kind: 'unknown-code', reason: 'unknown code ' + code } };
    }
    if (typeof payload !== 'object' || payload === null || Array.isArray(payload)) {
        return { ok: false, rejection: { kind: 'malformed', reason: `${code} needs an object payload` } };
    }
    const spec = paramSpecs[code];
    const record = payload as Record<string, unknown>;
    for (const key of Object.keys(record)) {
        if (!(key in spec)) {
            return { ok: false, rejection: { kind: 'malformed', reason: 'unknown key ' + key + ' for ' + code } };
        }
    }
    for (const [key, kind] of Object.entries(spec)) {
        const optional = kind.endsWith('?');
        const value = record[key];
        if (value === undefined || (optional && value === null)) {
            if (optional) {
                continue;
            }
            return { ok: false, rejection: { kind: 'malformed', reason: 'missing key ' + key + ' for ' + code } };
        }
        const expected = kind.startsWith('number') ? 'number' : 'string';
        if (typeof value !== expected || (expected === 'number' && !Number.isFinite(value as number))) {
            return { ok: false, rejection: { kind: 'malformed', reason: 'key ' + key + ' must be a ' + expected + ' for ' + code } };
        }
    }
    return { ok: true, params: record as OpenWordErrorParams };
}

/** Whether a code belongs to the frozen contract. */
export function isContractedCode(code: string): code is OpenWordErrorCode {
    return Object.prototype.hasOwnProperty.call(paramSpecs, code);
}

/**
 * Resolves one code and payload into a discriminated failure.
 *
 * Unknown codes and malformed payloads land on `binding/failure`, keeping the
 * original code and text as payload instead of guessing at parameters. That
 * fallback is itself structured, so every branch stays exhaustive.
 */
export function failureFrom(code: string, message: string, payload: unknown): OpenWordFailure {
    const parsed = parseErrorParams(code, payload);
    if (parsed.ok) {
        return { code: code as OpenWordErrorCode, params: parsed.params } as OpenWordFailure;
    }
    return { code: 'binding/failure', params: { reason: message, code } };
}

/** Options accepted by {@link OpenWordError}. */
export interface OpenWordErrorOptions {
    readonly recoverable?: boolean;
    readonly details?: OpenWordErrorDetails;
    readonly params?: unknown;
    readonly cause?: unknown;
}

/**
 * Payload the class derives when a caller omits one.
 *
 * Codes whose shape is only a `reason` take the message; codes whose shape is a
 * locator take it from `details`. Codes that address an object by `kind`/`id`
 * (or by `handle`, `path`, `operation`) must supply `params` explicitly, and an
 * omission stays unstructured so the failure surfaces as `binding/failure`
 * instead of a half-filled payload.
 */
function defaultParams(code: string, message: string, details: OpenWordErrorDetails): unknown {
    switch (code) {
        case 'session/argument-invalid':
        case 'format/invalid':
        case 'io/failure':
        case 'binding/failure':
            return { reason: message };
        case 'capability/not-implemented':
            return details.feature === undefined ? undefined : { feature: details.feature };
        case 'format/unsupported':
            return details.format === undefined ? undefined : { format: details.format };
        case 'document/revision-conflict':
            return details.revision === undefined ? undefined : { actual: details.revision };
        case 'session/disposed':
        case 'session/budget-exceeded':
        case 'document/not-found':
            return {};
        default:
            return undefined;
    }
}

/** Every failure thrown or rejected by this package. */
export class OpenWordError extends Error {
    /** The code and its frozen payload, as a discriminated union. */
    readonly failure: OpenWordFailure;
    /** Whether retrying with different input or state can succeed. */
    readonly recoverable: boolean;
    /** Optional locators (`revision`, `documentId`, `objectId`, `format`, `feature`). */
    readonly details: OpenWordErrorDetails;

    constructor(code: string, message: string, options: OpenWordErrorOptions = {}) {
        super(message, options.cause === undefined ? undefined : { cause: options.cause });
        this.name = 'OpenWordError';
        const params = options.params ?? defaultParams(code, message, options.details ?? {});
        this.failure = failureFrom(code, message, params);
        this.recoverable = options.recoverable ?? false;
        this.details = Object.freeze({ ...options.details });
    }

    /** Stable, comparable code. */
    get code(): OpenWordErrorCode {
        return this.failure.code;
    }

    /** Frozen parameters of {@link code}. */
    get params(): OpenWordErrorParams {
        return this.failure.params;
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
        super('capability/not-implemented', message, { recoverable: false, details: { feature }, params: { feature } });
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
 * carry yet; the machine-readable `code` and `params` never depend on locale.
 */
const messageCatalog: Readonly<Record<string, Readonly<Record<string, string>>>> = {
    en: {
        'session/disposed': 'the OpenWord session has been disposed; create a new instance',
        'session/argument-invalid': 'the request was rejected: {reason}',
        'session/budget-exceeded': 'the operation exceeded the session budget',
        'document/not-found': 'no live document has handle {handle}',
        'document/revision-conflict': 'the document changed at revision {actual}; the edit expected {expected}',
        'io/read': 'could not read `{path}` ({kind})',
        'io/write': 'could not publish `{path}`',
        'io/exists': '`{path}` already exists; pass overwrite to replace it',
        'io/failure': 'the file operation failed: {reason}',
        'object/not-found': 'the `{kind}` `{id}` was not found',
        'object/unsupported': 'the `{operation}` `{name}` is not supported by this build',
        'format/unsupported': '`{format}` has no route in this build',
        'format/invalid': 'the `{format}` input is invalid: {reason}',
        'capability/not-implemented': 'the `{feature}` capability is not implemented yet',
        'binding/failure': 'the native binding reported a failure: {reason}',
    },
    'zh-CN': {
        'session/disposed': 'OpenWord 会话已释放；请创建新实例',
        'session/argument-invalid': '请求被拒绝：{reason}',
        'session/budget-exceeded': '操作超出会话预算',
        'document/not-found': '没有句柄 {handle} 对应的活动文档',
        'document/revision-conflict': '文档已变更到修订 {actual}；本次编辑期望 {expected}',
        'io/read': '无法读取 `{path}`（{kind}）',
        'io/write': '无法发布 `{path}`',
        'io/exists': '`{path}` 已存在；传入 overwrite 以替换',
        'io/failure': '文件操作失败：{reason}',
        'object/not-found': '未找到 `{kind}` `{id}`',
        'object/unsupported': '此构建不支持 `{operation}` `{name}`',
        'format/unsupported': '`{format}` 在此构建中没有可用路径',
        'format/invalid': '`{format}` 输入无效：{reason}',
        'capability/not-implemented': '`{feature}` 能力尚未实现',
        'binding/failure': '原生绑定返回了失败：{reason}',
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
 * Unknown codes and missing translations fall back to the binding's own text,
 * so a replaced or removed native message never changes which branch a caller
 * takes: that branch is decided by `code` and `params`.
 */
export function renderErrorCopy(code: string, params: OpenWordErrorParams, fallback: string): string {
    const template = messageCatalog[currentLocale]?.[code];
    if (template === undefined) {
        return fallback;
    }
    const payload = params as Readonly<Record<string, unknown>>;
    let complete = true;
    const rendered = template.replace(/\{(\w+)\}/g, (match, key: string) => {
        if (payload[key] === undefined) {
            complete = false;
            return match;
        }
        return String(payload[key]);
    });
    return complete ? rendered : fallback;
}
