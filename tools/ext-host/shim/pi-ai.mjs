// `@earendil-works/pi-ai` 的最小面：StringEnum + Type（转发 typebox）。
import { Type } from "./typebox.mjs";

export { Type };
export function StringEnum(values) {
  return { type: "string", enum: values };
}
export const z = Type;
