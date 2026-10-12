//! Можно ли конвертировать PRIMARY: только если выделение принадлежит окну в фокусе или его
//! клиенту X11. Иначе PRIMARY мог остаться от другого окна, и его текст встал бы у каретки.

/// `id_mask` — `resource_id_mask` из handshake X11: биты вне маски — номер клиента, общий для
/// всех его окон. Окно неизвестно (0) — убедиться нельзя.
pub fn primary_belongs_to_focus(owner: u32, focused: u32, id_mask: u32) -> bool {
    owner != 0 && focused != 0 && (owner == focused || (owner & !id_mask) == (focused & !id_mask))
}
