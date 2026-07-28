# CarbonLedger Backend

NestJS API layer for the CarbonLedger Soroban smart-contract system.

## Validation architecture

All request bodies are validated through `GlobalValidationPipe` before they
reach a controller handler. The pipe is registered globally in `main.ts`.

### Request lifecycle

```
HTTP request
  └─ GlobalValidationPipe (class-validator + class-transformer)
       ├─ plainToInstance()   — parse JSON → typed DTO
       ├─ validate()          — run all decorators
       │    ├─ built-in decorators  (@IsString, @IsInt, @Min, @Max …)
       │    └─ custom decorators    (@IsValidStellarAddress, @IsValidSerialRange)
       └─ on error → 400 { statusCode, error, errors: [{ field, code, message }] }
Controller handler (only reached when validation passes)
```

### Error response format

```json
{
  "statusCode": 400,
  "error": "Bad Request",
  "errors": [
    {
      "field": "developerAddress",
      "code": "IS_VALID_STELLAR_ADDRESS",
      "message": "Must be a valid Stellar public key: starts with G, exactly 56 characters, base32 alphabet [A-Z2-7]."
    }
  ]
}
```

### Validation error catalog

| Code | Meaning |
|------|---------|
| `INVALID_STELLAR_ADDRESS` | Field must be a 56-char G-prefixed base32 Stellar public key |
| `SERIAL_RANGE_INVALID` | `serialEnd` ≥ `serialStart`, both > 0, width ≤ 1 000 000 000 |
| `VINTAGE_YEAR_OUT_OF_RANGE` | Integer in [2000, 2100] |
| `AMOUNT_MUST_BE_POSITIVE` | Integer ≥ 1 |
| `REQUIRED_FIELD_MISSING` | Field is required and must not be empty |
| `STRING_TOO_LONG` | Value exceeds field's max length |
| `PRICE_MUST_BE_POSITIVE` | USDC price in stroops, integer ≥ 1 |
| `SCORE_OUT_OF_RANGE` | `methodologyScore` integer in [0, 100] |

### Custom validators

#### `@IsValidStellarAddress()`

Validates `^G[A-Z2-7]{55}$` — structural check only (no checksum).
Located in `src/validation/stellar-address.validator.ts`.

#### `@IsValidSerialRange()`

Class-level decorator (applied to the DTO class, not a single property).
Checks that `serialEnd >= serialStart`, both > 0, and width ≤ 1 000 000 000.
Located in `src/validation/serial-range.validator.ts`.

### Adding a new validator

1. Create `src/validation/my-rule.validator.ts` implementing
   `ValidatorConstraintInterface`.
2. Add a matching entry to `VALIDATION_ERRORS` in
   `src/validation/validation-errors.ts`.
3. Export a decorator wrapping `registerDecorator`.
4. Apply the decorator to the relevant DTO field (or class).
5. Write tests in `src/validation/__tests__/my-rule.validator.spec.ts`.

## Running tests

```bash
npm test                 # all tests
npm test -- --coverage   # with coverage report
```

## Performance

`class-validator` compiles decorator metadata at module load time using a
`WeakMap` keyed on the DTO constructor. On warm paths validation adds well
under 1 ms of overhead per request, satisfying the <10 ms requirement.
