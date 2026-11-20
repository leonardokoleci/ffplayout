const PREFIX = '[ffplayout preview]'

export function previewLog(event: string, details: Record<string, unknown> = {}) {
    console.info(`${PREFIX} ${new Date().toISOString()} ${event}`, details)
}

export function previewNumber(value: number) {
    return Number.isFinite(value) ? Math.round(value * 1000) / 1000 : null
}
