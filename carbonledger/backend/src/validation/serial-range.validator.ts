import {
  registerDecorator,
  ValidationOptions,
  ValidatorConstraint,
  ValidatorConstraintInterface,
  ValidationArguments,
} from 'class-validator';
import { VALIDATION_ERRORS } from './validation-errors';

/** Maximum allowed serial range width (1 billion credits per batch). */
const MAX_SERIAL_RANGE_WIDTH = 1_000_000_000;

/**
 * Class-level validator: checks that `serialEnd >= serialStart`,
 * both values are positive, and the range width does not exceed the
 * system maximum.
 *
 * Apply to the DTO class (not a single property) using `@Validate` or
 * the `@IsValidSerialRange()` decorator below.
 */
@ValidatorConstraint({ name: 'IsValidSerialRange', async: false })
export class IsValidSerialRangeConstraint
  implements ValidatorConstraintInterface
{
  validate(_value: unknown, args: ValidationArguments): boolean {
    const obj = args.object as Record<string, unknown>;
    const start = obj['serialStart'];
    const end = obj['serialEnd'];

    if (typeof start !== 'number' || typeof end !== 'number') return false;
    if (!Number.isInteger(start) || !Number.isInteger(end)) return false;
    if (start <= 0 || end <= 0) return false;
    if (end < start) return false;
    if (end - start > MAX_SERIAL_RANGE_WIDTH) return false;
    return true;
  }

  defaultMessage(_args: ValidationArguments): string {
    return VALIDATION_ERRORS.SERIAL_RANGE_INVALID.message;
  }
}

/**
 * Decorator applied to a DTO **class** to validate the serial range formed by
 * `serialStart` and `serialEnd` properties.
 *
 * @example
 * \@IsValidSerialRange()
 * class MintCreditsDto { ... }
 */
export function IsValidSerialRange(
  validationOptions?: ValidationOptions,
): ClassDecorator {
  return function (target: Function): void {
    registerDecorator({
      target,
      propertyName: 'serialRange',
      options: {
        message: VALIDATION_ERRORS.SERIAL_RANGE_INVALID.message,
        ...validationOptions,
      },
      constraints: [],
      validator: IsValidSerialRangeConstraint,
    });
  };
}
