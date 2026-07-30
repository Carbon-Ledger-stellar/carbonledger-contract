import { IsString, IsNotEmpty, IsInt, Min, Max, MaxLength } from 'class-validator';
import { IsValidStellarAddress } from '../validation/stellar-address.validator';
import { VALIDATION_ERRORS } from '../validation/validation-errors';

/** DTO for registering a new carbon offset project. */
export class RegisterProjectDto {
  @IsValidStellarAddress({ message: VALIDATION_ERRORS.INVALID_STELLAR_ADDRESS.message })
  adminAddress!: string;

  @IsString() @IsNotEmpty() @MaxLength(64)
  projectId!: string;

  @IsString() @IsNotEmpty() @MaxLength(256)
  name!: string;

  @IsString() @IsNotEmpty() @MaxLength(256)
  metadataCid!: string;

  @IsValidStellarAddress({ message: VALIDATION_ERRORS.INVALID_STELLAR_ADDRESS.message })
  verifierAddress!: string;

  @IsString() @IsNotEmpty() @MaxLength(64)
  methodology!: string;

  @IsString() @IsNotEmpty() @MaxLength(128)
  country!: string;

  @IsString() @IsNotEmpty() @MaxLength(64)
  projectType!: string;

  @IsInt()
  @Min(2000, { message: VALIDATION_ERRORS.VINTAGE_YEAR_OUT_OF_RANGE.message })
  @Max(2100, { message: VALIDATION_ERRORS.VINTAGE_YEAR_OUT_OF_RANGE.message })
  vintageYear!: number;
}
