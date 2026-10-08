export class DownloadQueue {
  private readonly pending = new Map<string, () => void>()
  private readonly running = new Set<string>()

  constructor(private readonly parallelism: () => number) {}

  has(id: string): boolean {
    return this.pending.has(id) || this.running.has(id)
  }

  get isIdle(): boolean {
    return this.pending.size === 0 && this.running.size === 0
  }

  enqueue(id: string, start: () => void): boolean {
    if (this.has(id)) return false
    this.pending.set(id, start)
    this.refresh()
    return true
  }

  cancelPending(id: string): boolean {
    return this.pending.delete(id)
  }

  finish(id: string): void {
    this.running.delete(id)
    this.refresh()
  }

  refresh(): void {
    const value = this.parallelism()
    const limit = Number.isFinite(value) ? Math.max(1, Math.min(8, Math.round(value))) : 6
    while (this.pending.size > 0 && this.running.size < limit) {
      const [id, start] = this.pending.entries().next().value!
      this.pending.delete(id)
      this.running.add(id)
      start()
    }
  }
}
