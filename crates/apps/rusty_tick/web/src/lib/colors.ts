/** The swatches offered for lists and tags. */
export const SWATCHES = [
  '#e5312d', '#faa700', '#f5d400', '#0cce9c', '#4cb1ff', '#4772fa', '#8f6bff', '#ed70a5', '#a8a8a8', '#7b5b45', '#2f9e44', '#191919',
] as const

/** Accept `#rrggbb` only. */
export const isHexColor = (c: string): boolean => /^#[0-9a-f]{6}$/i.test(c)
