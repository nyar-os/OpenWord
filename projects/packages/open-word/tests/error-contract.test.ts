/**
 * Error contract checks against a stub binding.
 *
 * The stub replaces the native display text on purpose: the SDK must still
 * branch on the machine-readable `code` and typed `params`, never on prose.
 * Unknown codes, malformed payloads and the `binding/failure` fallback all run
 * through the same path.
 */

import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { isOpenWordError, parseErrorParams, setErrorLocale, type OpenWordError } from '../src/errors.js';
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

test('strict params reject missing, extra and mistyped keys', () => {
    assert.deepEqual(parseErrorParams('document/not-found', {}), { ok: true, params: {} }, 'a session without an open document names no handle');
    assert.equal(parseErrorParams('document/not-found', { handle: '7' }).ok, false, 'a handle must be a number');
    assert.equal(
        parseErrorParams('io/exists', { path: 'report.docx', legacy: true }).ok,
        false,
        'a key outside the frozen shape is refused',
    );
    assert.equal(
        parseErrorParams('document/revision-conflict', { actual: '7' }).ok,
        false,
        'a revision must be a number',
    );
    assert.equal(parseErrorParams('weird/code', {}).ok, false, 'an unknown code has no frozen shape');
    assert.deepEqual(parseErrorParams('object/not-found', { kind: 'block', id: '42' }), {
        ok: true,
        params: { kind: 'block', id: '42' },
    });
});

test('a malformed payload falls back to binding/failure with the original code and text', () => {
    process.env.OPEN_WORD_STUB = JSON.stringify({ message: 'disk said no', code: 'io/exists', detail: { params: 'not-an-object' } });
    const error = capture();
    assert.equal(error.code, 'binding/failure');
    assert.deepEqual(error.params, { reason: 'disk said no', code: 'io/exists' });
    assert.equal(error.message, 'the native binding reported a failure: disk said no', 'the binding text is kept as payload');
});

test('missing required params land on binding/failure', () => {
    process.env.OPEN_WORD_STUB = JSON.stringify({ message: 'no such block', code: 'object/not-found', detail: { params: { kind: 'block' } } });
    const error = capture();
    assert.equal(error.code, 'binding/failure');
    assert.deepEqual(error.params, { reason: 'no such block', code: 'object/not-found' });
});

test('prose that looks like a legacy code is not parsed', () => {
    process.env.OPEN_WORD_STUB = JSON.stringify({ message: 'document/not-found: old binding text' });
    const error = capture();
    assert.equal(error.code, 'binding/failure', 'a message never decides the code');
    assert.deepEqual(error.params, { reason: 'document/not-found: old binding text' });
});

test('unknown codes land on binding/failure and keep the original code in params', () => {
    process.env.OPEN_WORD_STUB = JSON.stringify({ message: 'native text kept', code: 'weird/code' });
    const error = capture();
    assert.equal(error.code, 'binding/failure');
    assert.deepEqual(error.params, { reason: 'native text kept', code: 'weird/code' });
});

test('a failure without any code surfaces as binding/failure', () => {
    process.env.OPEN_WORD_STUB = JSON.stringify({ message: 'boom' });
    const error = capture();
    assert.equal(error.code, 'binding/failure');
    assert.deepEqual(error.params, { reason: 'boom' });
    assert.equal(error.message, 'the native binding reported a failure: boom');
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

test('switching locale never moves the code or the params', () => {
    process.env.OPEN_WORD_STUB = JSON.stringify({
        message: 'REPLACED NATIVE TEXT',
        code: 'document/revision-conflict',
        detail: { params: { actual: 7, expected: 6 } },
    });
    const en = capture();
    setErrorLocale('zh-CN');
    try {
        const zh = capture();
        assert.equal(zh.code, en.code);
        assert.deepEqual(zh.params, en.params);
        assert.notEqual(zh.message, en.message, 'only the copy changes');
    } finally {
        setErrorLocale('en');
    }
});
