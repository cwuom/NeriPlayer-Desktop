export interface CoordinatedAuthMutation<T> {
  promise: Promise<T>
  isCurrent: () => boolean
}

export class AuthMutationRequestCoordinator {
  private generations = new Map<string, number>()
  private tails = new Map<string, Promise<void>>()

  run<T>(platform: string, loader: () => Promise<T> | T): CoordinatedAuthMutation<T> {
    const generation = (this.generations.get(platform) ?? 0) + 1
    this.generations.set(platform, generation)

    const previous = this.tails.get(platform) ?? Promise.resolve()
    const promise = previous.then(loader)
    const tail = promise.then(
      () => undefined,
      () => undefined,
    )

    this.tails.set(platform, tail)
    void tail.finally(() => {
      if (this.tails.get(platform) === tail) {
        this.tails.delete(platform)
      }
    })

    return {
      promise,
      isCurrent: () => this.generations.get(platform) === generation,
    }
  }
}
