export interface NeteaseLibraryRequestResult {
  current: boolean
  playlistsOk: boolean
  albumsOk: boolean
}

export interface CoordinatedNeteaseLibraryRequest {
  started: boolean
  promise: Promise<NeteaseLibraryRequestResult>
}

type NeteaseLibraryLoader = () => Promise<boolean>

export class NeteaseLibraryRequestCoordinator {
  private generation = 0
  private inFlight: Promise<NeteaseLibraryRequestResult> | null = null

  invalidate(): void {
    this.generation += 1
    this.inFlight = null
  }

  run(
    loadPlaylists: NeteaseLibraryLoader,
    loadAlbums: NeteaseLibraryLoader,
  ): CoordinatedNeteaseLibraryRequest {
    if (this.inFlight) {
      return { started: false, promise: this.inFlight }
    }

    const generation = this.generation
    const playlists = this.load(loadPlaylists)
    const albums = this.load(loadAlbums)
    let request!: Promise<NeteaseLibraryRequestResult>
    request = Promise.all([playlists, albums])
      .then(([playlistsOk, albumsOk]) => ({
        current: this.generation === generation,
        playlistsOk,
        albumsOk,
      }))
      .finally(() => {
        if (this.inFlight === request) {
          this.inFlight = null
        }
      })

    this.inFlight = request
    return { started: true, promise: request }
  }

  private load(loader: NeteaseLibraryLoader): Promise<boolean> {
    try {
      return Promise.resolve(loader()).then(
        value => value,
        () => false,
      )
    } catch {
      return Promise.resolve(false)
    }
  }
}
