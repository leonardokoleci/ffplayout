import { nextTick } from 'vue'

const FLASH_CLASS = 'playlist-item-flash'

/**
 * Briefly flash a highlight over the rows with the given element ids so
 * users can see which playlist rows were just inserted, moved, or pasted.
 * The highlight is an animated overlay that fades back to the row's own
 * background color.
 */
export async function flashRows(ids: string[]) {
    await nextTick()

    for (const id of ids) {
        const row = document.getElementById(id)

        if (!row) continue

        row.classList.remove(FLASH_CLASS)
        // Force a reflow, so a repeated flash restarts the animation.
        void row.offsetWidth
        row.classList.add(FLASH_CLASS)
        row.addEventListener('animationend', () => row.classList.remove(FLASH_CLASS), { once: true })
    }
}
