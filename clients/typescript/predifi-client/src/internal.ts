export function isPlainObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

export function parseJson(text: string): unknown {
  if (text.trim() === '') {
    return undefined;
  }
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return undefined;
  }
}

export function isErrorEnvelope(value: unknown): boolean {
  return isPlainObject(value) && value.status === 'error';
}

export function unwrapEnvelope(value: unknown): unknown {
  if (isPlainObject(value) && value.status === 'success' && 'data' in value) {
    return value.data;
  }
  return value;
}

export function isUnsupportedVersion(value: unknown): boolean {
  if (!isPlainObject(value)) {
    return false;
  }
  const source = isPlainObject(value.error) ? value.error : value;
  return source.code === 'UNSUPPORTED_API_VERSION' || source.code === 'API_VERSION_NOT_SUPPORTED';
}
