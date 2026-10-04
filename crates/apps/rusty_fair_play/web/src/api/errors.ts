/** Failures the store knows how to react to. Adapters translate transport details into these. */
import type { Card } from './types'

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message)
    this.name = 'ApiError'
  }
}

/** 401: the token is missing or wrong. */
export class UnauthorizedError extends ApiError {
  constructor(message = 'missing or invalid bearer token') {
    super(401, 'unauthorized', message)
    this.name = 'UnauthorizedError'
  }
}

/** 404 */
export class NotFoundError extends ApiError {
  constructor(message = 'not found') {
    super(404, 'not_found', message)
    this.name = 'NotFoundError'
  }
}

/** 400/422: the request was refused as malformed or invalid. Retrying will not help. */
export class InvalidError extends ApiError {
  constructor(status: number, message: string) {
    super(status, status === 422 ? 'invalid' : 'bad_request', message)
    this.name = 'InvalidError'
  }
}

/** 409: the id or name is already taken. */
export class ConflictError extends ApiError {
  constructor(message: string) {
    super(409, 'conflict', message)
    this.name = 'ConflictError'
  }
}

/** 412: the `If-Match` etag is not the card's any more; `current` is the card as stored now. */
export class StaleError extends ApiError {
  constructor(
    readonly current: Card,
    message = 'the card changed since it was read',
  ) {
    super(412, 'precondition_failed', message)
    this.name = 'StaleError'
  }
}

/** The server could not be reached. The operation may be retried later. */
export class NetworkError extends Error {
  constructor(message = 'the server could not be reached') {
    super(message)
    this.name = 'NetworkError'
  }
}
