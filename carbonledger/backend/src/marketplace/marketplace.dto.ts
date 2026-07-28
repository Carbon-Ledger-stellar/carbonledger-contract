import { IsString, IsNotEmpty, IsInt, Min, Max, MaxLength } from 'class-validator';
import { IsValidStellarAddress } from '../validation/stellar-address.validator';
import { VALIDATION_ERRORS } from '../validation/validation-errors';

/** DTO for creating a marketplace listing. */
export class ListCreditsDto {
  @IsValidStellarAddress({ message: VALIDATION_ERRORS.INVALID_STELLAR_ADDRESS.message })
  sellerId!: string;

  @IsString() @IsNotEmpty() @MaxLength(64)
  listingId!: string;

  @IsString() @IsNotEmpty() @MaxLength(64)
  batchId!: string;

  @IsString() @IsNotEmpty() @MaxLength(64)
  projectId!: string;

  @IsInt() @Min(1, { message: VALIDATION_ERRORS.AMOUNT_MUST_BE_POSITIVE.message })
  amount!: number;

  @IsInt() @Min(1, { message: VALIDATION_ERRORS.PRICE_MUST_BE_POSITIVE.message })
  pricePerCreditUsdc!: number;

  @IsInt()
  @Min(2000, { message: VALIDATION_ERRORS.VINTAGE_YEAR_OUT_OF_RANGE.message })
  @Max(2100, { message: VALIDATION_ERRORS.VINTAGE_YEAR_OUT_OF_RANGE.message })
  vintageYear!: number;

  @IsString() @IsNotEmpty() @MaxLength(64)
  methodology!: string;

  @IsString() @IsNotEmpty() @MaxLength(128)
  country!: string;
}

/** DTO for purchasing credits from a marketplace listing. */
export class PurchaseCreditsDto {
  @IsValidStellarAddress({ message: VALIDATION_ERRORS.INVALID_STELLAR_ADDRESS.message })
  buyerAddress!: string;

  @IsString() @IsNotEmpty() @MaxLength(64)
  listingId!: string;

  @IsInt() @Min(1, { message: VALIDATION_ERRORS.AMOUNT_MUST_BE_POSITIVE.message })
  amount!: number;
}

/** DTO for de-listing (cancelling) an active marketplace listing. */
export class DelistCreditsDto {
  @IsValidStellarAddress({ message: VALIDATION_ERRORS.INVALID_STELLAR_ADDRESS.message })
  sellerAddress!: string;

  @IsString() @IsNotEmpty() @MaxLength(64)
  listingId!: string;
}
