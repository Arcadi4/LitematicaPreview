// Shared apt repository contract for the Linux release packages and the publisher.

export const APT_SOURCE_PATH = "/etc/apt/sources.list.d/litematica-preview.list"
export const APT_KEYRING_PATH = "/usr/share/keyrings/litematica-preview-archive-keyring.gpg"
export const APT_DISTRIBUTION = "stable"
export const APT_COMPONENT = "main"
export const APT_ARCHITECTURE = "amd64"
export const APT_PACKAGE = "litematica-preview"

export function normalizeAptBaseUrl(value: string): string {
  let url: URL
  try {
    url = new URL(value)
  } catch {
    throw new Error(`The apt repository base URL is not a URL: ${value}`)
  }
  if (url.protocol !== "https:") {
    throw new Error(`The apt repository base URL must use https: ${value}`)
  }
  if (url.username || url.password) {
    throw new Error("The apt repository base URL must not contain credentials.")
  }
  if (url.search || url.hash) {
    throw new Error("The apt repository base URL must not contain a query or fragment.")
  }
  if (url.pathname !== "/") {
    throw new Error("The apt repository base URL must point to the bucket root.")
  }
  return url.toString().replace(/\/$/, "")
}

export function renderAptSource(baseUrl: string): string {
  return `deb [arch=${APT_ARCHITECTURE} signed-by=${APT_KEYRING_PATH}] ${normalizeAptBaseUrl(baseUrl)} ${APT_DISTRIBUTION} ${APT_COMPONENT}\n`
}
