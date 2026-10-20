// Hash saved synthetic drafts using the same canonical payload as the product.
import { canonicalPayload } from "../src/transcript-state.ts";
export async function stampFixtureDraft(draft) {
  if (draft) {
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(JSON.stringify(canonicalPayload(draft.payload))));
    draft.contentHash = Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, "0")).join("");
  }
  return structuredClone(draft);
}
