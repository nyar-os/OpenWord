/**
 * Error contract checks against a stub binding.
 *
 * The stub replaces the native display text on purpose: the SDK must still
 * branch on the machine-readable `code` and typed `params`, never on prose.
 * Unknown codes, missing translations, broken payloads and the legacy
 * `code: message` adapter all run through the same path.
 */

import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { isOpenWordError, setErrorLocale, type OpenWordError } from '../src/errors.js';
import * as service from '../src/service.js';

const dir = mkdtempSync(join(tmpdir(), 'open-word-stub-'));
const stubPath = join(dir, 'stub-binding.cjs');
writeFileSync(
    stubPath,
    [
        'function failure() {',
        '    const next = JSON.parse(process.env.OPEN_WORD_STUB || "{}");',
        '    const error = new Error(next.message || "the native text was replaced");',
        '    if (next.code !== undefined) { error.code = next.code; }',
        '    if (next.detail !== undefined) { error.detail = next.detail; }',
        '    return error;',
        '}',
        'module.exports = {',
        '    version() { return "0.0.0-stub"; },',
        '    document_create() { throw failure(); },',
        '};',
    ].join('\n'),
);
process.env.OPEN_WORD_NATIVE = stubPath;

function capture(): OpenWordError {
    try {
        service.create({ title: 'stub' });
    } catch (error) {
        assert.ok(isOpenWordError(error), 'every service failure is an OpenWordError');
        return error;
    }
    return assert.fail('the stub always throws') as never;
}

test('structured binding failures keep their code and typed params', () => {
    process.env.OPEN_WORD_STUB = JSON.stringify({
        message: 'REPLACED NATIVE TEXT',
        code: 'object/not-found',
        detail: { params: { kind: 'block', id: '42' } },
    });
    const error = capture();
    assert.equal(error.code, 'object/not-found');
    assert.deepEqual(error.params, { kind: 'block', id: '42' });
    assert.equal(error.message, 'the `block` `42` was not found', 'the product copy replaces the native text');
});

test('unknown codes are preserved instead of being misclassified', () => {
    process.env.OPEN_WORD_STUB = JSON.stringify({ message: 'native text kept', code: 'weird/code' });
    const error = capture();
    assert.equal(error.code, 'weird/code');
    assert.equal(error.message, 'native text kept', 'a missing translation falls back to the binding text');
});

test('a broken params payload degrades to an empty parameter map', () => {
    process.env.OPEN_WORD_STUB = JSON.stringify({ message: 'disk said no', code: 'io/exists', detail: { params: 'not-an-object' } });
    const error = capture();
    assert.equal(error.code, 'io/exists');
    assert.deepEqual(error.params, {});
    assert.equal(error.message, 'disk said no', 'an unrenderable template keeps the binding text');
});

test('legacy `code: message` concatenation still decodes at the adapter entry', () => {
    process.env.OPEN_WORD_STUB = JSON.stringify({ message: 'document/not-found: old binding text' });
    const error = capture();
    assert.equal(error.code, 'document/not-found');
    assert.equal(error.message, 'old binding text');
});

test('a failure without any code surfaces as binding/failure', () => {
    process.env.OPEN_WORD_STUB = JSON.stringify({ message: 'boom' });
    const error = capture();
    assert.equal(error.code, 'binding/failure');
    assert.equal(error.message, 'boom');
});

test('zh-CN rendering changes only the copy, never the machine fields', () => {
    setErrorLocale('zh-CN');
    try {
        process.env.OPEN_WORD_STUB = JSON.stringify({
            message: 'REPLACED NATIVE TEXT',
            code: 'object/not-found',
            detail: { params: { kind: 'block', id: '42' } },
        });
        const error = capture();
        assert.equal(error.code, 'object/not-found');
        assert.deepEqual(error.params, { kind: 'block', id: '42' });
        assert.equal(error.message, '未找到 `block` `42`');
    } finally {
        setErrorLocale('en');
    }
});
