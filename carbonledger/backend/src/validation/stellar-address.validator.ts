import {
  registerDecorator,
  ValidationOptions,
  ValidatorConstraint,
  ValidatorConstraintInterface,
  ValidationArguments,
} from 'class-validator';
import { VALIDATION_ERRORS } from './validation-errors';

/**
 * Validates that a string is a well-formed Stellar public key:
 *   - Starts with 'G'
 *   - Exactly 56 characters long
 *   - Only base32 alphabet characters: A-Z and 2-7
 *
 * Note: this is a structural check, not a checksum verification.
 * Full Ed25519 key validation (StrKey decode + checksum) would require
 * a Stellar SDK dependency and is out of scope for the middleware layer.
 */
@ValidatorConstraint({ name: 'IsValidStellarAddress', async: false })
export class IsValidStellarAddressConstraint
  implements ValidatorConstraintInterface
{
  private static readonly STELLAR_ADDRESS_REGEX = /^G[A-Z2-7]{55}$/;

  validate(value: unknown, _args: ValidationArguments): boolean {
    if (typeof value !== 'string') return false;
    return IsValidStellarAddressConstraint.STELLAR_ADDRESS_REGEX.test(value);
  }

  defaultMessage(_args: ValidationArguments): string {
    return VALIDATION_ERRORS.INVALID_STELLAR_ADDRESS.message;
  }
}

/**
 * Decorator: ensures the field contains a structurally valid Stellar
 * public key (G + 55 base32 chars).
 *
 * @example
 * \@IsValidStellarAddress()
 * developerAddress: string;
 */
export function IsValidStellarAddress(
  validationOptions?: ValidationOptions,
): PropertyDecorator {
  return function (object: object, propertyName: string | symbol): void {
    registerDecorator({
      target: object.constructor,
      propertyName: propertyName as string,
      options: {
        message: VALIDATION_ERRORS.INVALID_STELLAR_ADDRESS.message,
        ...validationOptions,
      },
      constraints: [],
      validator: IsValidStellarAddressConstraint,
    });
  };
}
