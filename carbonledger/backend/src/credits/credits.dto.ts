import {
  IsString,
  IsNotEmpty,
  IsInt,
  Min,
  Max,
  MaxLength,
} from 'class-validator';
import { IsValidStellarAddress } from '../validation/stellar-address.validator';
import { IsValidSerialRange } from '../validation/serial-range.validator';
import { VALIDATION_ERRORS } from '../validation/validation-errors';

/**
 * DTO for minting a new carbon credit batch.
 * Maps 1-to-1 to `carbon_credit::mint_credits`.
 */
@IsValidSerialRange()
export class MintCreditsDto {
  @IsString({ message: VALIDATION_ERRORS.REQUIRED_FIELD_MISSING.message })
  @IsNotEmpty({ message: VALIDATION_ERRORS.REQUIRED_FIELD_MISSING.message })
  @MaxLength(64, { message: VALIDATION_ERRORS.STRING_TOO_LONG.message })
  projectId!: string;

  @IsInt({ message: VALIDATION_ERRORS.AMOUNT_MUST_BE_POSITIVE.message })
  @Min(1, { message: VALIDATION_ERRORS.AMOUNT_MUST_BE_POSITIVE.message })
  amount!: number;

  @IsInt({ message: VALIDATION_ERRORS.VINTAGE_YEAR_OUT_OF_RANGE.message })
  @Min(2000, { message: VALIDATION_ERRORS.VINTAGE_YEAR_OUT_OF_RANGE.message })
  @Max(2100, { message: VALIDATION_ERRORS.VINTAGE_YEAR_OUT_OF_RANGE.message })
  vintageYear!: number;

  @IsString({ message: VALIDATION_ERRORS.REQUIRED_FIELD_MISSING.message })
  @IsNotEmpty({ message: VALIDATION_ERRORS.REQUIRED_FIELD_MISSING.message })
  @MaxLength(64, { message: VALIDATION_ERRORS.STRING_TOO_LONG.message })
  batchId!: string;

  @IsInt({ message: VALIDATION_ERRORS.SERIAL_RANGE_INVALID.message })
  @Min(1, { message: VALIDATION_ERRORS.SERIAL_RANGE_INVALID.message })
  serialStart!: number;

  @IsInt({ message: VALIDATION_ERRORS.SERIAL_RANGE_INVALID.message })
  @Min(1, { message: VALIDATION_ERRORS.SERIAL_RANGE_INVALID.message })
  serialEnd!: number;

  @IsString({ message: VALIDATION_ERRORS.REQUIRED_FIELD_MISSING.message })
  @IsNotEmpty({ message: VALIDATION_ERRORS.REQUIRED_FIELD_MISSING.message })
  @MaxLength(256, { message: VALIDATION_ERRORS.STRING_TOO_LONG.message })
  metadataCid!: string;

  @IsValidStellarAddress({
    message: VALIDATION_ERRORS.INVALID_STELLAR_ADDRESS.message,
  })
  developerAddress!: string;
}

/**
 * DTO for permanently retiring carbon credits on-chain.
 * Maps to `carbon_credit::retire_credits`.
 */
export class RetireCreditsDto {
  @IsString()
  @IsNotEmpty({ message: VALIDATION_ERRORS.REQUIRED_FIELD_MISSING.message })
  @MaxLength(64, { message: VALIDATION_ERRORS.STRING_TOO_LONG.message })
  batchId!: string;

  @IsInt({ message: VALIDATION_ERRORS.AMOUNT_MUST_BE_POSITIVE.message })
  @Min(1, { message: VALIDATION_ERRORS.AMOUNT_MUST_BE_POSITIVE.message })
  amount!: number;

  @IsString()
  @IsNotEmpty({ message: VALIDATION_ERRORS.REQUIRED_FIELD_MISSING.message })
  @MaxLength(512, { message: VALIDATION_ERRORS.STRING_TOO_LONG.message })
  retirementReason!: string;

  @IsString()
  @IsNotEmpty({ message: VALIDATION_ERRORS.REQUIRED_FIELD_MISSING.message })
  @MaxLength(256, { message: VALIDATION_ERRORS.STRING_TOO_LONG.message })
  beneficiary!: string;

  @IsString()
  @IsNotEmpty({ message: VALIDATION_ERRORS.REQUIRED_FIELD_MISSING.message })
  @MaxLength(64, { message: VALIDATION_ERRORS.STRING_TOO_LONG.message })
  retirementId!: string;

  @IsString()
  @IsNotEmpty({ message: VALIDATION_ERRORS.REQUIRED_FIELD_MISSING.message })
  @MaxLength(128, { message: VALIDATION_ERRORS.STRING_TOO_LONG.message })
  txHash!: string;

  @IsValidStellarAddress({
    message: VALIDATION_ERRORS.INVALID_STELLAR_ADDRESS.message,
  })
  retiredByAddress!: string;
}

/**
 * DTO for transferring carbon credits between Stellar addresses.
 * Maps to `carbon_credit::transfer_credits`.
 */
export class TransferCreditsDto {
  @IsValidStellarAddress({
    message: VALIDATION_ERRORS.INVALID_STELLAR_ADDRESS.message,
  })
  fromAddress!: string;

  @IsValidStellarAddress({
    message: VALIDATION_ERRORS.INVALID_STELLAR_ADDRESS.message,
  })
  toAddress!: string;

  @IsString()
  @IsNotEmpty({ message: VALIDATION_ERRORS.REQUIRED_FIELD_MISSING.message })
  @MaxLength(64, { message: VALIDATION_ERRORS.STRING_TOO_LONG.message })
  batchId!: string;

  @IsInt({ message: VALIDATION_ERRORS.AMOUNT_MUST_BE_POSITIVE.message })
  @Min(1, { message: VALIDATION_ERRORS.AMOUNT_MUST_BE_POSITIVE.message })
  amount!: number;
}
