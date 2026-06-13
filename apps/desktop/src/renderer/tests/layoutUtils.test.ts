import { describe, it, expect } from 'vitest';
import { tilingColumns } from '../src/lib/layoutUtils';

describe('tilingColumns', () => {
  it('returns 1 for count=0', () => {
    expect(tilingColumns(0)).toBe(1);
  });

  it('returns 1 for count=1', () => {
    expect(tilingColumns(1)).toBe(1);
  });

  it('returns 2 for count=2', () => {
    expect(tilingColumns(2)).toBe(2);
  });

  it('returns 2 for count=3', () => {
    expect(tilingColumns(3)).toBe(2);
  });

  it('returns 2 for count=4', () => {
    expect(tilingColumns(4)).toBe(2);
  });

  it('returns 3 for count=5', () => {
    expect(tilingColumns(5)).toBe(3);
  });

  it('returns 3 for count=6', () => {
    expect(tilingColumns(6)).toBe(3);
  });

  it('returns ceil(sqrt(7))=3 for count=7', () => {
    expect(tilingColumns(7)).toBe(3);
  });

  it('returns ceil(sqrt(8))=3 for count=8', () => {
    expect(tilingColumns(8)).toBe(3);
  });

  it('returns ceil(sqrt(9))=3 for count=9', () => {
    expect(tilingColumns(9)).toBe(3);
  });

  it('returns ceil(sqrt(10))=4 for count=10', () => {
    expect(tilingColumns(10)).toBe(4);
  });

  it('returns ceil(sqrt(16))=4 for count=16', () => {
    expect(tilingColumns(16)).toBe(4);
  });

  it('returns ceil(sqrt(25))=5 for count=25', () => {
    expect(tilingColumns(25)).toBe(5);
  });
});
