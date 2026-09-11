/** sessionStorage key for the Playground virtual key field. */
export const PLAYGROUND_VK_STORAGE_KEY = "playground_vk";

/** Persist a plaintext key so Playground can pick it up on next open. */
export function stashPlaygroundVirtualKey(plaintext: string): void {
  sessionStorage.setItem(PLAYGROUND_VK_STORAGE_KEY, plaintext);
}
