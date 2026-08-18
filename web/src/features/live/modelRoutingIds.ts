const SAFE_MODEL_ID_CHARACTERS = /^[a-wy-zA-WY-Z0-9_-]$/;

/**
 * Keep common model IDs readable while escaping every other code point.
 * The sentinel is escaped too, so the resulting CSS identifier is collision-free.
 */
export function modelRoutingKey(model: string) {
  return Array.from(model, (character) => {
    if (SAFE_MODEL_ID_CHARACTERS.test(character)) return character;
    return `x${character.codePointAt(0)?.toString(16)}x`;
  }).join("");
}
