import 'reflect-metadata';
import { validate, plainToInstance } from 'class-validator';
import { MintCreditsDto } from '../../credits.dto';

const VALID_ADDRESS = 'GAAZI4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN';

function validPayload(): Record<string, unknown> {
  return {
    projectId: 'proj-001',
    amount: 1000,
    vintageYear: 2023,
    batchId: 'batch-001',
    serialStart: 1,
    serialEnd: 1000,
    metadataCid: 'QmCIDHash',
    developerAddress: VALID_ADDRESS,
  };
}

async function validateDto(plain: Record<string, unknown>) {
  const dto = plainToInstance(MintCreditsDto, plain);
  return validate(dto);
}

describe('MintCreditsDto', () => {
  it('passes with a fully valid payload', async () => {
    const errors = await validateDto(validPayload());
    expect(errors).toHaveLength(0);
  });

  it('fails when projectId is missing', async () => {
    const { projectId: _, ...rest } = validPayload();
    const errors = await validateDto(rest);
    const fields = errors.map((e) => e.property);
    expect(fields).toContain('projectId');
  });

  it('fails when projectId is empty string', async () => {
    const errors = await validateDto({ ...validPayload(), projectId: '' });
    expect(errors.some((e) => e.property === 'projectId')).toBe(true);
  });

  it('fails when amount is 0', async () => {
    const errors = await validateDto({ ...validPayload(), amount: 0 });
    expect(errors.some((e) => e.property === 'amount')).toBe(true);
  });

  it('fails when amount is negative', async () => {
    const errors = await validateDto({ ...validPayload(), amount: -5 });
    expect(errors.some((e) => e.property === 'amount')).toBe(true);
  });

  it('fails when vintageYear is 1999 (below range)', async () => {
    const errors = await validateDto({ ...validPayload(), vintageYear: 1999 });
    expect(errors.some((e) => e.property === 'vintageYear')).toBe(true);
  });

  it('fails when vintageYear is 2101 (above range)', async () => {
    const errors = await validateDto({ ...validPayload(), vintageYear: 2101 });
    expect(errors.some((e) => e.property === 'vintageYear')).toBe(true);
  });

  it('passes with vintageYear at boundary 2000', async () => {
    const errors = await validateDto({ ...validPayload(), vintageYear: 2000 });
    expect(errors).toHaveLength(0);
  });

  it('passes with vintageYear at boundary 2100', async () => {
    const errors = await validateDto({ ...validPayload(), vintageYear: 2100 });
    expect(errors).toHaveLength(0);
  });

  it('fails when developerAddress is not a valid Stellar address', async () => {
    const errors = await validateDto({
      ...validPayload(),
      developerAddress: 'not-a-stellar-address',
    });
    expect(errors.some((e) => e.property === 'developerAddress')).toBe(true);
  });

  it('fails when developerAddress starts with S (secret key)', async () => {
    const secretKey = 'SAAZI4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN';
    const errors = await validateDto({
      ...validPayload(),
      developerAddress: secretKey,
    });
    expect(errors.some((e) => e.property === 'developerAddress')).toBe(true);
  });

  it('fails when serialEnd < serialStart (invalid range)', async () => {
    const errors = await validateDto({
      ...validPayload(),
      serialStart: 500,
      serialEnd: 100,
    });
    // The class-level @IsValidSerialRange decorator reports on property 'serialRange'
    expect(errors.some((e) => e.property === 'serialRange')).toBe(true);
  });

  it('fails when serialStart is 0', async () => {
    const errors = await validateDto({
      ...validPayload(),
      serialStart: 0,
      serialEnd: 1000,
    });
    // Caught by both field-level @Min(1) and class-level @IsValidSerialRange
    expect(
      errors.some((e) => e.property === 'serialStart' || e.property === 'serialRange'),
    ).toBe(true);
  });

  it('fails when metadataCid is empty', async () => {
    const errors = await validateDto({ ...validPayload(), metadataCid: '' });
    expect(errors.some((e) => e.property === 'metadataCid')).toBe(true);
  });

  it('fails when batchId exceeds 64 characters', async () => {
    const errors = await validateDto({
      ...validPayload(),
      batchId: 'b'.repeat(65),
    });
    expect(errors.some((e) => e.property === 'batchId')).toBe(true);
  });

  it('error messages reference the correct VALIDATION_ERRORS catalog codes', async () => {
    const errors = await validateDto({
      ...validPayload(),
      developerAddress: 'bad-address',
    });
    const addrError = errors.find((e) => e.property === 'developerAddress');
    expect(addrError).toBeDefined();
    const messages = Object.values(addrError!.constraints ?? {});
    expect(messages.some((m) => m.includes('Stellar'))).toBe(true);
  });
});
