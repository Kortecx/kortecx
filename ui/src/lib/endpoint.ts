/**
 * Endpoint validation for the connection form. The cleartext-token warning is the
 * SDK's `isNonloopbackPlaintext` (re-exported so the UI and SDK agree exactly).
 */

import { isNonloopbackPlaintext } from "@kortecx/sdk/web";

export { isNonloopbackPlaintext };

/** Return a human error message for an invalid endpoint, or `null` if it is valid. */
export function validateEndpoint(endpoint: string): string | null {
  const e = endpoint.trim();
  if (e === "") {
    return "endpoint is required";
  }
  if (!/^https?:\/\//.test(e)) {
    return "endpoint must start with http:// or https://";
  }
  try {
    new URL(e);
  } catch {
    return "endpoint is not a valid URL";
  }
  return null;
}
