const BADGE_CLASS = 'multi-drag-badge'

// Dedicated payload types keep native drags distinguishable: media file
// drops use the generic JSON type shared with the folder tree, while row
// drags carry their ids in list specific types.
export const MEDIA_DRAG_TYPE = 'application/json'
export const PLAYLIST_ROW_DRAG_TYPE = 'application/x-playlist-uids'
export const RUNDOWN_ROW_DRAG_TYPE = 'application/x-rundown-id'

function createBadge(label: string) {
    const badge = document.createElement('div')
    badge.className = BADGE_CLASS
    badge.textContent = label

    return badge
}

/**
 * Native drag flow (media page): the browser captures the drag image from
 * the grabbed element right after dragstart, so a short-lived badge on the
 * element ends up in the snapshot.
 */
export function multiDragGhost(event: DragEvent, count: number, label: string) {
    if (count < 2) return

    const item = (event.target as HTMLElement | null)?.closest<HTMLElement>('li, tr')

    if (item && !item.querySelector(`.${BADGE_CLASS}`)) {
        const badge = createBadge(label)
        badge.style.position = 'absolute'
        badge.style.top = '0.1rem'
        badge.style.right = '0.25rem'
        badge.style.zIndex = '10'
        badge.style.pointerEvents = 'none'
        badge.style.boxShadow = '0 0.25rem 0.5rem #0008'

        if (getComputedStyle(item).position === 'static') {
            item.style.position = 'relative'
        }

        item.appendChild(badge)
        setTimeout(() => badge.remove(), 250)
    }

    if (event.dataTransfer) {
        const ghost = document.createElement('div')
        ghost.textContent = label
        ghost.style.position = 'absolute'
        ghost.style.top = '-10000px'
        ghost.style.padding = '0.35rem 0.75rem'
        ghost.style.borderRadius = '0.5rem'
        ghost.style.background = '#111827'
        ghost.style.color = '#fff'
        ghost.style.fontSize = '0.8rem'
        ghost.style.fontWeight = '600'

        document.body.appendChild(ghost)
        event.dataTransfer.setDragImage(ghost, 12, 12)

        setTimeout(() => ghost.remove())
    }
}

/**
 * Native HTML5 drops from the media browsers carry no sortable metadata, so
 * the insertion point comes from the pointer: the row under the cursor,
 * before it in the upper half and after it in the lower half. Without a row
 * the drop lands at the end of the list.
 */
export function nativeDropIndex(event: DragEvent, items: { uid: string }[]): number {
    const target = document.elementFromPoint(event.clientX, event.clientY)
    const row = target instanceof Element ? target.closest<HTMLElement>('li') : null

    if (!row) return items.length

    const index = items.findIndex((item) => item.uid === row.id)

    if (index === -1) return items.length

    const rect = row.getBoundingClientRect()

    return event.clientY > rect.top + rect.height / 2 ? index + 1 : index
}

// Auto-scroll state for the active native drag. Native drag and drop gives
// no reliable scrolling of its own, so dragover feeds the pointer position
// to updateDragScroll and a rAF loop scrolls the nearest scrollable
// ancestor while the pointer hovers near its edges.
const AUTO_SCROLL_EDGE = 48
const AUTO_SCROLL_MAX_SPEED = 14

let scrollFrame = 0
let scrollScroller: HTMLElement | null = null
let scrollSpeed = 0

function scrollableAncestor(node: Element | null): HTMLElement | null {
    let current = node as HTMLElement | null

    while (current && current !== document.body) {
        const overflowY = getComputedStyle(current).overflowY

        if ((overflowY === 'auto' || overflowY === 'scroll') && current.scrollHeight > current.clientHeight) {
            return current
        }

        current = current.parentElement
    }

    return null
}

function scrollStep() {
    scrollFrame = 0

    if (!scrollScroller || !scrollSpeed) return

    scrollScroller.scrollTop += scrollSpeed
    scrollFrame = requestAnimationFrame(scrollStep)
}

export function updateDragScroll(event: DragEvent) {
    const target = document.elementFromPoint(event.clientX, event.clientY)
    const scroller = scrollableAncestor(target)

    if (!scroller || scroller !== scrollScroller) {
        stopDragScroll()
    }

    if (!scroller) return

    const rect = scroller.getBoundingClientRect()
    const topDistance = event.clientY - rect.top
    const bottomDistance = rect.bottom - event.clientY
    let speed = 0

    if (topDistance < AUTO_SCROLL_EDGE) {
        speed = -Math.ceil(((AUTO_SCROLL_EDGE - topDistance) / AUTO_SCROLL_EDGE) * AUTO_SCROLL_MAX_SPEED)
    } else if (bottomDistance < AUTO_SCROLL_EDGE) {
        speed = Math.ceil(((AUTO_SCROLL_EDGE - bottomDistance) / AUTO_SCROLL_EDGE) * AUTO_SCROLL_MAX_SPEED)
    }

    if (!speed) {
        stopDragScroll()

        return
    }

    scrollScroller = scroller
    scrollSpeed = speed

    if (!scrollFrame) {
        scrollFrame = requestAnimationFrame(scrollStep)
    }
}

export function stopDragScroll() {
    if (scrollFrame) {
        cancelAnimationFrame(scrollFrame)
    }

    scrollFrame = 0
    scrollScroller = null
    scrollSpeed = 0
}
