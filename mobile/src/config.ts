export const SERVER_URL = 'http://100.68.192.63:5533';
export const POLL_INTERVAL_MS = 1000;

// Kept for api.ts compatibility
export function getApiBaseUrl(): string {
  return SERVER_URL;
}
