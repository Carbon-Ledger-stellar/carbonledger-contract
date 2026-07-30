import { IsString, IsNotEmpty, IsInt, Min, Max, MaxLength } from 'class-validator';
import { IsValidStellarAddress } from '../validation/stellar-address.validator';
import { VALIDATION_ERRORS } from '../validation/validation-errors';

/** DTO for submitting satellite monitoring data for a project period. */
export class SubmitMonitoringDto {
  @IsValidStellarAddress({ message: VALIDATION_ERRORS.INVALID_STELLAR_ADDRESS.message })
  oracleSigner!: string;

  @IsString() @IsNotEmpty() @MaxLength(64)
  projectId!: string;

  @IsString() @IsNotEmpty() @MaxLength(32)
  period!: string;

  @IsInt() @Min(1, { message: VALIDATION_ERRORS.AMOUNT_MUST_BE_POSITIVE.message })
  tonnesVerified!: number;

  @IsInt()
  @Min(0, { message: VALIDATION_ERRORS.SCORE_OUT_OF_RANGE.message })
  @Max(100, { message: VALIDATION_ERRORS.SCORE_OUT_OF_RANGE.message })
  methodologyScore!: number;

  @IsString() @IsNotEmpty() @MaxLength(256)
  satelliteCid!: string;
}

/** DTO for submitting a benchmark price update. */
export class UpdatePriceDto {
  @IsValidStellarAddress({ message: VALIDATION_ERRORS.INVALID_STELLAR_ADDRESS.message })
  oracleSigner!: string;

  @IsString() @IsNotEmpty() @MaxLength(64)
  methodology!: string;

  @IsInt()
  @Min(2000, { message: VALIDATION_ERRORS.VINTAGE_YEAR_OUT_OF_RANGE.message })
  @Max(2100, { message: VALIDATION_ERRORS.VINTAGE_YEAR_OUT_OF_RANGE.message })
  vintageYear!: number;

  @IsInt() @Min(1, { message: VALIDATION_ERRORS.PRICE_MUST_BE_POSITIVE.message })
  priceUsdc!: number;
}
