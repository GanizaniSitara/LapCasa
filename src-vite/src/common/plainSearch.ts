import { invoke } from '@tauri-apps/api/core';
import { ref } from 'vue';

let revision = 0;
let cancellation: Promise<unknown> = Promise.resolve();
export const plainSearchRevision = () => revision;
export const waitForPlainSearchCancellation = () => cancellation;
export const plainSearchSubmission = ref(0);

export function submitPlainSearch() {
  plainSearchSubmission.value++;
}

// Invalidate rendered responses immediately, including during the typing debounce.
export function cancelPlainSearch() {
  revision++;
  cancellation = cancellation.then(() => invoke('cancel_plain_search'))
    .catch(error => console.error('Cancel search:', error));
}
