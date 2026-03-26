//! Box-drawing characters and junction merge logic.
//!
//! Shared by all renderers (scanline, arena, classic). Provides:
//! - Unicode box-drawing character constants (light weight)
//! - Direction bitmask encoding for junction resolution
//! - `merge_chars`: composites overlapping box-drawing characters into proper junctions
//!
//! # Direction Bitmask Encoding
//!
//! Each box-drawing character is encoded as a 4-bit mask indicating which
//! directions it connects:
//!
//! ```text
//! bit 0 (1) = Up
//! bit 1 (2) = Down
//! bit 2 (4) = Left
//! bit 3 (8) = Right
//! ```
//!
//! For example, `└` (CORNER_DR) connects Up and Right = `1 | 8 = 9`.
//!
//! When two characters overlap at the same cell, their masks are OR'd together
//! and the result maps back to the appropriate junction character. This produces
//! correct T-junctions and crossings automatically.
//!
//! # Future: Line Weights
//!
//! This module currently only defines light-weight characters (`─ │ ┌ ┐ └ ┘`).
//! Heavy (`━ ┃ ┏ ┓`), double (`═ ║ ╔ ╗`), and dashed (`┈ ┊`) weights can be
//! added as separate constant groups. The `merge_chars` logic would then track
//! per-direction weights and select from Unicode's mixed-weight junction characters
//! (e.g., `╂` for light-vertical + heavy-horizontal).

// ── Light-weight box-drawing characters ──────────────────────────────────

/// Vertical line: `│`
pub const V_LINE: char = '│';
/// Horizontal line: `─`
pub const H_LINE: char = '─';
/// Downward arrow: `↓`
pub const ARROW_DOWN: char = '↓';

// ── Dashed characters for reversed (back) edges ─────────────────────────
// Mirrors zigraph's `CP_V_LINE_DASH`, `CP_H_LINE_DASH`, etc.

/// Dashed vertical line: `┊` (light quadruple dash vertical)
pub const V_LINE_DASHED: char = '┊';
/// Dashed horizontal line: `┈` (light quadruple dash horizontal)
pub const H_LINE_DASHED: char = '┈';
/// Dashed downward arrow: `⇣`
pub const ARROW_DOWN_DASHED: char = '⇣';
/// Dashed upward arrow: `⇡`
pub const ARROW_UP_DASHED: char = '⇡';
/// Self-loop indicator: `↺`
pub const SELF_LOOP: char = '↺';

/// Corner: down-right `└` (connects Up + Right)
pub const CORNER_DR: char = '└';
/// Corner: down-left `┘` (connects Up + Left)
pub const CORNER_DL: char = '┘';
/// Corner: up-right `┌` (connects Down + Right)
pub const CORNER_UR: char = '┌';
/// Corner: up-left `┐` (connects Down + Left)
pub const CORNER_UL: char = '┐';

/// Cross junction: `┼` (all four directions)
pub const CROSS: char = '┼';

/// T-junction pointing down: `┬` (Down + Left + Right)
pub const TEE_DOWN: char = '┬';
/// T-junction pointing up: `┴` (Up + Left + Right)
pub const TEE_UP: char = '┴';
/// T-junction pointing right: `├` (Up + Down + Right)
pub const TEE_RIGHT: char = '├';
/// T-junction pointing left: `┤` (Up + Down + Left)
pub const TEE_LEFT: char = '┤';

// ── Direction bitmask constants ──────────────────────────────────────────

/// Bitmask: connects upward
pub const DIR_UP: u8 = 1;
/// Bitmask: connects downward
pub const DIR_DOWN: u8 = 2;
/// Bitmask: connects left
pub const DIR_LEFT: u8 = 4;
/// Bitmask: connects right
pub const DIR_RIGHT: u8 = 8;

// ── Bitmask ↔ character conversion ───────────────────────────────────────

/// Encode a box-drawing character as a direction bitmask.
///
/// Returns 0 for characters that aren't box-drawing (except `ARROW_DOWN`
/// which connects upward).
#[inline]
pub fn char_direction_mask(c: char) -> u8 {
    match c {
        V_LINE | V_LINE_DASHED => DIR_UP | DIR_DOWN,
        H_LINE | H_LINE_DASHED => DIR_LEFT | DIR_RIGHT,
        CORNER_DR => DIR_UP | DIR_RIGHT,
        CORNER_DL => DIR_UP | DIR_LEFT,
        CORNER_UR => DIR_DOWN | DIR_RIGHT,
        CORNER_UL => DIR_DOWN | DIR_LEFT,
        TEE_UP => DIR_UP | DIR_LEFT | DIR_RIGHT,
        TEE_DOWN => DIR_DOWN | DIR_LEFT | DIR_RIGHT,
        TEE_LEFT => DIR_UP | DIR_DOWN | DIR_LEFT,
        TEE_RIGHT => DIR_UP | DIR_DOWN | DIR_RIGHT,
        CROSS => DIR_UP | DIR_DOWN | DIR_LEFT | DIR_RIGHT,
        ARROW_DOWN | ARROW_DOWN_DASHED => DIR_UP, // Arrow tip connects upward to the line above
        ARROW_UP_DASHED => DIR_DOWN,              // Upward arrow connects downward
        _ => 0,
    }
}

/// Decode a direction bitmask back to the appropriate box-drawing character.
///
/// Falls back through progressively simpler characters if the exact mask
/// doesn't match a specific junction.
#[inline]
pub fn mask_to_char(mask: u8) -> char {
    match mask {
        // Exact matches
        3 => V_LINE,
        12 => H_LINE,
        9 => CORNER_DR,
        5 => CORNER_DL,
        10 => CORNER_UR,
        6 => CORNER_UL,
        13 => TEE_UP,
        14 => TEE_DOWN,
        7 => TEE_LEFT,
        11 => TEE_RIGHT,
        15 => CROSS,
        // Fallbacks: pick the best match when extra bits are set
        m if (m & 15) == 15 => CROSS,
        m if (m & 13) == 13 => TEE_UP,
        m if (m & 14) == 14 => TEE_DOWN,
        m if (m & 7) == 7 => TEE_LEFT,
        m if (m & 11) == 11 => TEE_RIGHT,
        m if (m & 9) == 9 => CORNER_DR,
        m if (m & 5) == 5 => CORNER_DL,
        m if (m & 10) == 10 => CORNER_UR,
        m if (m & 6) == 6 => CORNER_UL,
        m if (m & 12) == 12 => H_LINE,
        m if (m & 3) == 3 => V_LINE,
        1 | 2 => V_LINE,
        4 | 8 => H_LINE,
        _ => ' ',
    }
}

/// Merge two overlapping box-drawing characters into the correct junction.
///
/// Rules:
/// 1. Space yields to any character
/// 2. Identical characters stay unchanged
/// 3. Arrows take precedence (they're endpoints)
/// 4. Direction masks are OR'd and decoded back to a junction character
///
/// # Examples
///
/// ```
/// use ascii_dag::render::chars::merge_chars;
///
/// // Vertical + horizontal = cross
/// assert_eq!(merge_chars('│', '─'), '┼');
///
/// // Space yields
/// assert_eq!(merge_chars(' ', '│'), '│');
/// assert_eq!(merge_chars('─', ' '), '─');
///
/// // Arrow wins
/// assert_eq!(merge_chars('│', '↓'), '↓');
/// ```
#[inline]
pub fn merge_chars(c1: char, c2: char) -> char {
    if c1 == ' ' {
        return c2;
    }
    if c2 == ' ' {
        return c1;
    }
    if c1 == c2 {
        return c1;
    }
    if c1 == ARROW_DOWN || c2 == ARROW_DOWN {
        return ARROW_DOWN;
    }
    if c1 == ARROW_DOWN_DASHED || c2 == ARROW_DOWN_DASHED {
        return ARROW_DOWN_DASHED;
    }
    if c1 == ARROW_UP_DASHED || c2 == ARROW_UP_DASHED {
        return ARROW_UP_DASHED;
    }

    let m1 = char_direction_mask(c1);
    let m2 = char_direction_mask(c2);
    if m1 == 0 {
        return c2;
    }
    if m2 == 0 {
        return c1;
    }

    let union = m1 | m2;
    let merged = mask_to_char(union);
    if merged == ' ' { c1 } else { merged }
}

/// Convert a solid box-drawing character to its dashed equivalent.
///
/// Used when rendering reversed (back) edges with dashed lines.
/// Characters without a dashed variant are returned unchanged.
#[inline]
pub fn to_dashed(c: char) -> char {
    match c {
        V_LINE => V_LINE_DASHED,
        H_LINE => H_LINE_DASHED,
        ARROW_DOWN => ARROW_DOWN_DASHED,
        _ => c, // corners, tees, etc. have no dashed variant — keep solid
    }
}
