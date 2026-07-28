import { validate } from 'class-validator';
import { IsValidStellarAddressConstraint } from '../stellar-address.validator';

// Bare-minimum shim so the test file runs without a real DI container.
class TestDto {
  constructor(public address: string) {}
}

function makeConstraint() {
  return new IsValidStellarAddressConstraint();
}

const VALID_ADDRESS = 'GAAZI4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN';
const VALID_ADDRESS_2 = 'GCKFBEIYV2U22IO2BJ4KVJOIP7XPWQGQFKKWXR6DOSJBV7STMAQSMTM';

describe('IsValidStellarAddressConstraint', () => {
  let constraint: IsValidStellarAddressConstraint;

  beforeEach(() => {
    constraint = makeConstraint();
  });

  it('accepts a well-formed G-address (56 chars, base32)', () => {
    expect(constraint.validate(VALID_ADDRESS, {} as any)).toBe(true);
  });

  it('accepts a second valid G-address', () => {
    expect(constraint.validate(VALID_ADDRESS_2, {} as any)).toBe(true);
  });

  it('rejects an address that does not start with G', () => {
    const bad = 'AAAZI4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN';
    expect(constraint.validate(bad, {} as any)).toBe(false);
  });

  it('rejects an address that is too short (55 chars)', () => {
    const bad = VALID_ADDRESS.slice(0, 55);
    expect(constraint.validate(bad, {} as any)).toBe(false);
  });

  it('rejects an address that is too long (57 chars)', () => {
    const bad = VALID_ADDRESS + 'A';
    expect(constraint.validate(bad, {} as any)).toBe(false);
  });

  it('rejects an address containing invalid base32 char (0)', () => {
    // Replace a valid char with '0' which is not in [A-Z2-7]
    const bad = VALID_ADDRESS.slice(0, 10) + '0' + VALID_ADDRESS.slice(11);
    expect(constraint.validate(bad, {} as any)).toBe(false);
  });

  it('rejects an address containing invalid base32 char (lowercase)', () => {
    const bad = 'Gaazi4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN';
    expect(constraint.validate(bad, {} as any)).toBe(false);
  });

  it('rejects null', () => {
    expect(constraint.validate(null, {} as any)).toBe(false);
  });

  it('rejects undefined', () => {
    expect(constraint.validate(undefined, {} as any)).toBe(false);
  });

  it('rejects an empty string', () => {
    expect(constraint.validate('', {} as any)).toBe(false);
  });

  it('rejects a number', () => {
    expect(constraint.validate(12345, {} as any)).toBe(false);
  });

  it('provides a descriptive default error message', () => {
    const msg = constraint.defaultMessage({} as any);
    expect(msg).toContain('Stellar');
    expect(msg).toContain('56');
  });
});
