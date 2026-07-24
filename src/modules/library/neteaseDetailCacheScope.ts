export interface NeteaseDetailAuthIdentity {
  loggedIn: boolean
  accountId: string | null
}

function normalizeAccountId(value: string | null): string {
  return typeof value === 'string' ? value.trim().normalize('NFC') : ''
}

function encodeAccountId(value: string): string {
  return Array.from(new TextEncoder().encode(value))
    .map(byte => byte.toString(16).padStart(2, '0'))
    .join('')
}

export function createNeteaseDetailCacheScope(
  auth: NeteaseDetailAuthIdentity,
  sessionVersion: number,
): string | null {
  if (!auth.loggedIn) return null

  const accountId = normalizeAccountId(auth.accountId)
  if (!accountId) return null
  const normalizedSessionVersion = Number.isSafeInteger(sessionVersion) && sessionVersion >= 0
    ? sessionVersion
    : 0

  return `account-${encodeAccountId(accountId)}:session-${normalizedSessionVersion}`
}
