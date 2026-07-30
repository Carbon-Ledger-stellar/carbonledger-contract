import { IsValidSerialRangeConstraint } from '../serial-range.validator';

function makeArgs(serialStart: unknown, serialEnd: unknown): any {
  return { object: { serialStart, serialEnd } };
}

describe('IsValidSerialRangeConstraint', () => {
  let constraint: IsValidSerialRangeConstraint;

  beforeEach(() => {
    constraint = new IsValidSerialRangeConstraint();
  });

  it('accepts a valid range serialStart=1, serialEnd=1000', () => {
    expect(constraint.validate(null, makeArgs(1, 1000))).toBe(true);
  });

  it('accepts a single-credit range serialStart=1, serialEnd=1', () => {
    expect(constraint.validate(null, makeArgs(1, 1))).toBe(true);
  });

  it('accepts range exactly at the 1 billion width limit', () => {
    expect(constraint.validate(null, makeArgs(1, 1_000_000_000))).toBe(true);
  });

  it('rejects range one over the 1 billion width limit', () => {
    expect(constraint.validate(null, makeArgs(1, 1_000_000_001))).toBe(false);
  });

  it('rejects inverted range (serialEnd < serialStart)', () => {
    expect(constraint.validate(null, makeArgs(100, 50))).toBe(false);
  });

  it('rejects serialStart = 0', () => {
    expect(constraint.validate(null, makeArgs(0, 100))).toBe(false);
  });

  it('rejects serialEnd = 0', () => {
    expect(constraint.validate(null, makeArgs(1, 0))).toBe(false);
  });

  it('rejects negative serialStart', () => {
    expect(constraint.validate(null, makeArgs(-1, 100))).toBe(false);
  });

  it('rejects negative serialEnd', () => {
    expect(constraint.validate(null, makeArgs(1, -100))).toBe(false);
  });

  it('rejects non-integer serialStart (float)', () => {
    expect(constraint.validate(null, makeArgs(1.5, 100))).toBe(false);
  });

  it('rejects non-integer serialEnd (float)', () => {
    expect(constraint.validate(null, makeArgs(1, 100.9))).toBe(false);
  });

  it('rejects undefined fields', () => {
    expect(constraint.validate(null, makeArgs(undefined, undefined))).toBe(false);
  });

  it('provides a descriptive default error message', () => {
    const msg = constraint.defaultMessage({} as any);
    expect(msg).toContain('serialEnd');
    expect(msg).toContain('serialStart');
  });
});
