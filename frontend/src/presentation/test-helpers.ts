// Inputs are synthetic JSON DTOs; freezing catches accidental writes at any depth.
export function freezeFixture<T>(value: T): T {
  if (value !== null && typeof value === "object") {
    Object.values(value).forEach(freezeFixture);
    Object.freeze(value);
  }
  return value;
}
