/**
 * Centralised validation error catalog.
 *
 * Every validation failure maps to one of these entries.  The `code` is
 * machine-readable and stable; the `message` is human-readable and included
 * in the HTTP 400 response body so clients can display actionable guidance.
 */
export const VALIDATION_ERRORS = {
  INVALID_STELLAR_ADDRESS: {
    code: 'INVALID_STELLAR_ADDRESS',
    message:
      'Must be a valid Stellar public key: starts with G, exactly 56 characters, base32 alphabet [A-Z2-7].',
    httpStatus: 400,
  },
  SERIAL_RANGE_INVALID: {
    code: 'SERIAL_RANGE_INVALID',
    message:
      'serialEnd must be ≥ serialStart, both must be > 0, and the range width must not exceed 1,000,000,000.',
    httpStatus: 400,
  },
  VINTAGE_YEAR_OUT_OF_RANGE: {
    code: 'VINTAGE_YEAR_OUT_OF_RANGE',
    message: 'vintageYear must be an integer between 2000 and 2100 inclusive.',
    httpStatus: 400,
  },
  AMOUNT_MUST_BE_POSITIVE: {
    code: 'AMOUNT_MUST_BE_POSITIVE',
    message: 'amount must be a positive integer (≥ 1).',
    httpStatus: 400,
  },
  REQUIRED_FIELD_MISSING: {
    code: 'REQUIRED_FIELD_MISSING',
    message: 'This field is required and must not be empty.',
    httpStatus: 400,
  },
  STRING_TOO_LONG: {
    code: 'STRING_TOO_LONG',
    message: 'The value exceeds the maximum allowed length for this field.',
    httpStatus: 400,
  },
  PRICE_MUST_BE_POSITIVE: {
    code: 'PRICE_MUST_BE_POSITIVE',
    message: 'price must be a positive integer (≥ 1) representing USDC stroops.',
    httpStatus: 400,
  },
  SCORE_OUT_OF_RANGE: {
    code: 'SCORE_OUT_OF_RANGE',
    message: 'methodologyScore must be an integer between 0 and 100 inclusive.',
    httpStatus: 400,
  },
} as const;

export type ValidationErrorCode = keyof typeof VALIDATION_ERRORS;
