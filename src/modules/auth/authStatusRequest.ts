export interface AuthStatusRequestResult<T> {
  current: boolean
  value: T
}

export interface CoordinatedAuthStatusRequest<T> {
  started: boolean
  promise: Promise<AuthStatusRequestResult<T>>
}

export class AuthStatusRequestCoordinator<T> {
  private generation = 0
  private inFlight: Promise<AuthStatusRequestResult<T>> | null = null

  run(loader: () => Promise<T>): CoordinatedAuthStatusRequest<T> {
    if (this.inFlight) {
      return { started: false, promise: this.inFlight }
    }

    const generation = this.generation
    let request!: Promise<AuthStatusRequestResult<T>>
    request = Promise.resolve()
      .then(loader)
      .then(value => ({ current: generation === this.generation, value }))
      .finally(() => {
        if (this.inFlight === request) this.inFlight = null
      })

    this.inFlight = request
    return { started: true, promise: request }
  }

  invalidate(): void {
    this.generation++
    this.inFlight = null
  }
}
