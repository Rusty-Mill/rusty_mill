---
category: Fixed
changelog: Restore rleval queue test overrides safely across dynamic proxy cleanup.
---
## 2026-10-28 - Isolate rleval queue tests

- **Fixed:** preserve the application queue factory and Celery eager setting when
  queue tests finish, avoiding missing state in later service tests with pytest's
  attribute cleanup. Regression checks cover a normal upload after injected-queue
  cleanup and restoration of both enabled and disabled eager settings.
- **Known limitations:** validation uses in-process scoring and mocked AWS services;
  it does not exercise a live broker or cloud endpoints.
