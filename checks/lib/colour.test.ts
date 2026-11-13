import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { contrast, over, parseColour, themeValues } from './colour.ts';

describe('colour', () => {
  it('computes WCAG contrast', () => {
    assert.equal(contrast(parseColour('#000'), parseColour('#ffffff')), 21);
    assert.equal(contrast(parseColour('#777'), parseColour('#777')), 1);
  });

  it('composites translucent layers', () => {
    const c = over(parseColour('rgba(0, 0, 0, 0.5)'), parseColour('#ffffff'));
    assert.ok(Math.abs(c.r - 127.5) < 0.01 && c.a === 1);
  });

  it('splits light-dark() values', () => {
    assert.deepEqual(themeValues('light-dark(rgba(9, 9, 11, 0.05), #fff)'), {
      light: 'rgba(9, 9, 11, 0.05)',
      dark: '#fff',
    });
    assert.deepEqual(themeValues('#71717a'), { light: '#71717a', dark: '#71717a' });
  });
});
