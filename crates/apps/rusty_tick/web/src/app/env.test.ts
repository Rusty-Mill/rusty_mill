import { beforeEach, describe, expect, it } from 'vitest'
import { adoptIdentity, forgetUserData, identity } from './env'

const seed = () => {
  localStorage.setItem('server:tick-local:cache:v1', 'a')
  localStorage.setItem('tick-local:prefs:v1', 'a')
  localStorage.setItem('tick-local:ui:v1', 'layout')
  localStorage.setItem('tick-local:memory:v1', 'demo')
}
const left = () => Object.keys(localStorage).sort()

describe('identity', () => {
  it('is the user key of a per-user token, empty for the single-user token', () => {
    expect(identity('alice.s3cret')).toBe('alice')
    expect(identity('a-single-user-token')).toBe('')
    expect(identity(null)).toBe('')
  })
})

describe('adoptIdentity', () => {
  beforeEach(seed)

  it('keeps the cache for the same user, and for the single-user token', () => {
    expect(adoptIdentity('tok-without-dot')).toBe(false)
    expect(localStorage.getItem('server:tick-local:cache:v1')).toBe('a')
    adoptIdentity('alice.x')
    seed()
    expect(adoptIdentity('alice.other-device')).toBe(false)
    expect(localStorage.getItem('server:tick-local:cache:v1')).toBe('a')
  })

  it("drops the last user's cache and preferences when another user arrives, keeping layout and demo data", () => {
    adoptIdentity('alice.x')
    seed()
    expect(adoptIdentity('bob.y')).toBe(true)
    expect(left()).toEqual(['tick-local:memory:v1', 'tick-local:ui:v1', 'tick-local:who'])
  })

  it('treats data from before identities existed as a single user, so a named user starts clean', () => {
    expect(adoptIdentity('alice.x')).toBe(true)
    expect(localStorage.getItem('server:tick-local:cache:v1')).toBeNull()
  })
})

describe('forgetUserData', () => {
  it('leaves the token, mode and layout alone', () => {
    seed()
    localStorage.setItem('tick-local:token', 't')
    localStorage.setItem('tick-local:mode', 'server')
    forgetUserData()
    expect(left()).toEqual(['tick-local:memory:v1', 'tick-local:mode', 'tick-local:token', 'tick-local:ui:v1'])
  })
})
