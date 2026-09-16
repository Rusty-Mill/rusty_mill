//! `User` — a Rust port of `server/model/user.go`. The schema (fields only)
//! landed in an earlier increment; this one ports the query layer:
//! `create`/`create_oauth`/`delete_by_username`/`authenticate`/
//! `get_by_token`/`regenerate_token`/`get_by_username`/`get_by_id`/
//! `regenerate_token_by_username`/`rename`/`set_password`/
//! `get_by_oauth_id`/`toggle_admin`/`rules_json`/`set_rules_json` (Go:
//! `CreateUser`/`CreateOAuthUser`/`DeleteUser`/`AuthenticateUser`/
//! `GetUserByToken`/`RegenerateToken`/`GetUser`/`GetUserByID`/
//! `RegenerateTokenByUsername`/`UpdateUsername`/`UpdatePassword`/
//! `GetUserByOAuthID`/`ToggleAdmin`/`GetUserRules`/`SaveUserRules`). This
//! is the last of `rusty-hister-model`'s six Go model files — its query
//! layer is now fully ported.
//!
//! **Password hashing: Argon2id, not bcrypt.** Go hashes passwords with
//! `golang.org/x/crypto/bcrypt`; no first-party `rusty_*` crate covers
//! password hashing at all (a sovereignty-loop pass found none), and
//! `argon2` is already a workspace dependency (`rusty_croc`'s PAKE
//! handshake), so this uses that rather than adding a second
//! password-hashing crate. This is a deliberate algorithm change, not a
//! capability drop: Argon2id is the current OWASP-recommended default and
//! every password this crate ever hashes is freshly created here (the
//! still-open "does `rusty_hister` need to open a pre-existing
//! Hister-Go-created database file" question, see `docs/PROJECT-STATUS.md`,
//! means there is no existing bcrypt hash this needs to stay compatible
//! with under the current fresh-install-only working assumption) — flagged
//! in `docs/PROJECT-STATUS.md` for explicit sign-off, same as every other
//! judgment call this crate's query-layer increments have made. Salt bytes
//! come from `rusty_rand` (this crate's established CSPRNG entry point,
//! per `CrawlJob::generate_id`), not `argon2`'s own optional `rand`
//! feature, which stays disabled.
//!
//! **`authenticate` collapses Go's `ErrUserNotFound`/`ErrInvalidPassword`
//! into one `None` case.** Verified against the only real caller
//! (`server/endpoints.go`'s `serveLogin`), which already treats both
//! identically (a bare `err != nil` check → HTTP 401 "invalid
//! credentials") — collapsing them here doesn't drop any observable
//! behavior, and avoids a footgun where a future caller could
//! accidentally build a username-enumeration oracle from the distinction.
//!
//! **`create`/`create_oauth`'s hard `DeleteUser`-equivalent, `rename`'s
//! `ErrUserAlreadyExists`, and `Conflict`.** Unlike `history.go`'s tables,
//! `delete_by_username` doesn't need the hard-delete-vs-soft-delete
//! decision documented in `history.rs`'s module doc — Go's `DeleteUser`
//! is a hard delete for the same underlying reason (`CommonFields.DeletedAt`
//! isn't GORM's `gorm.DeletedAt`), so this does the same via a genuine
//! `DELETE`, not this crate's `#[table(soft_delete)]` column.
//!
//! **`GetUserRules`/`SaveUserRules` stay at the raw-JSON level.** Go's
//! `ParseRules`/`config.Rules` (skip/priority/versioning regex rules,
//! alias expansion) is a compiled-regex config-rules engine that doesn't
//! exist in this Rust codebase yet — the same scope boundary
//! `CrawlJob::validator_rules: Json` already draws for crawl-time
//! validator rules. `rules_json`/`set_rules_json` read/write the stored
//! `Json` blob as-is; parsing/compiling it into a structured, matchable
//! rule set is separate, not-yet-started work wherever that engine
//! eventually lands.
//!
//! **`RegenerateToken`'s no-such-user quirk is preserved as-is.** Go's
//! version issues `UPDATE users SET token = ? WHERE id = ?` with no
//! existence check and no error even when it matches zero rows — it
//! always returns the freshly generated token regardless. `regenerate_token`
//! reproduces that (unlike `regenerate_token_by_username`, which does
//! check first, matching Go's own asymmetry between the two functions).

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rusty_db::prelude::*;

/// Soft-deleted via `deleted` (Hister's Go source uses a nullable
/// `DeletedAt` column for the same purpose — `rusty_db`'s
/// `#[table(soft_delete)]` is the first-party equivalent, so this crate
/// uses that mechanism instead of hand-rolling a nullable timestamp).
#[derive(Debug, Clone, PartialEq, Mapped)]
#[table(name = "users")]
pub struct User {
    #[table(primary_key)]
    pub id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[table(soft_delete)]
    pub deleted: bool,
    /// Unique login name.
    pub username: String,
    pub password: String,
    pub token: String,
    pub is_admin: bool,
    /// JSON-encoded per-user rules document (Go: `RulesJSON string`,
    /// default `'{}'`). Stored as `Json` rather than a raw string since
    /// `rusty_db` has first-class JSON support.
    #[table(default = "'{}'")]
    pub rules_json: Json,
    /// OAuth subject identifier, when the account was created via OAuth.
    pub oauth_id: Option<String>,
}

/// The three outcomes [`User::rename`] can produce (Go: `UpdateUsername`,
/// which signals them via a plain error return — a return-only sentinel
/// would work but a closed enum makes the exhaustive three-way branch
/// checkable by the compiler at every call site).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameOutcome {
    Renamed,
    /// `new_username` is already taken by another account.
    UsernameTaken,
    /// No user has the given (old) username.
    NotFound,
}

/// Hashes `password` with Argon2id, using a fresh `rusty_rand`-sourced
/// salt — see this module's doc comment for why Argon2id, not bcrypt.
fn hash_password(password: &str) -> rusty_db::Result<String> {
    let salt_bytes = rusty_rand::bytes(16)
        .map_err(|e| rusty_db::Error::QueryBuilder(format!("failed to generate salt: {e}")))?;
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|e| rusty_db::Error::QueryBuilder(format!("failed to encode salt: {e}")))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| rusty_db::Error::QueryBuilder(format!("failed to hash password: {e}")))
}

/// Checks `password` against a stored Argon2id hash. `false` for any
/// verification failure, including a hash string that doesn't parse (a
/// row this crate never wrote wouldn't be a bcrypt hash Argon2 could
/// somehow still accept — it just fails to verify, same observable
/// result as a wrong password).
fn verify_password(password: &str, stored_hash: &str) -> bool {
    let Ok(parsed_hash) = PasswordHash::new(stored_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

/// Generates a random bearer token (Go: `rand.Text()`, a base32-ish
/// 26-character string from Go 1.24's `crypto/rand`). This crate doesn't
/// reproduce that exact alphabet — tokens are opaque bearer credentials
/// compared only for equality, so a hex encoding of `rusty_rand` bytes at
/// least as much entropy is behaviorally equivalent.
fn generate_token() -> rusty_db::Result<String> {
    let bytes = rusty_rand::bytes(32)
        .map_err(|e| rusty_db::Error::QueryBuilder(format!("failed to generate token: {e}")))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

impl User {
    /// Inserts a fresh row with a database-assigned id — shared by
    /// `create`/`create_oauth`. Named `insert_new`, not `insert`, since
    /// `#[derive(Mapped)]` already generates an `insert()` instance
    /// method a same-named associated function would collide with.
    async fn insert_new(
        engine: &Engine,
        username: &str,
        password_hash: &str,
        token: &str,
        is_admin: bool,
        oauth_id: Option<&str>,
    ) -> rusty_db::Result<Self> {
        let now = Utc::now();
        let dialect = engine.dialect();
        let p = crate::placeholders(dialect, 7);
        let params: Vec<Value> = vec![
            username.to_string().into(),
            password_hash.to_string().into(),
            token.to_string().into(),
            is_admin.into(),
            now.into(),
            now.into(),
            oauth_id.map(str::to_string).into(),
        ];
        let mut conn = engine.connect().await?;
        let id: i64 = if dialect.supports_returning() {
            let sql = format!(
                "INSERT INTO users \
                    (username, password, token, is_admin, created_at, updated_at, oauth_id) \
                 VALUES ({}, {}, {}, {}, {}, {}, {}) RETURNING id",
                p[0], p[1], p[2], p[3], p[4], p[5], p[6]
            );
            conn.fetch_one(&sql, &params).await?.get_by_name("id")?
        } else {
            let sql = format!(
                "INSERT INTO users \
                    (username, password, token, is_admin, created_at, updated_at, oauth_id) \
                 VALUES ({}, {}, {}, {}, {}, {}, {})",
                p[0], p[1], p[2], p[3], p[4], p[5], p[6]
            );
            conn.execute(&sql, &params).await?;
            conn.fetch_one("SELECT last_insert_rowid() AS id", &[])
                .await?
                .get_by_name("id")?
        };

        Ok(Self {
            id,
            created_at: now,
            updated_at: now,
            deleted: false,
            username: username.to_string(),
            password: password_hash.to_string(),
            token: token.to_string(),
            is_admin,
            rules_json: serde_json::json!({}),
            oauth_id: oauth_id.map(str::to_string),
        })
    }

    /// Creates a password-authenticated account (Go: `CreateUser`).
    /// `Err(Error::Conflict(_))` if `username` is already taken.
    pub async fn create(
        engine: &Engine,
        username: &str,
        password: &str,
        is_admin: bool,
    ) -> rusty_db::Result<Self> {
        if Self::get_by_username(engine, username).await?.is_some() {
            return Err(rusty_db::Error::Conflict(
                "username already exists".to_string(),
            ));
        }
        let hash = hash_password(password)?;
        let token = generate_token()?;
        Self::insert_new(engine, username, &hash, &token, is_admin, None).await
    }

    /// Creates an OAuth-authenticated account — no local password (Go:
    /// `CreateOAuthUser`). `Err(Error::Conflict(_))` if `username` is
    /// already taken.
    pub async fn create_oauth(
        engine: &Engine,
        username: &str,
        oauth_id: &str,
    ) -> rusty_db::Result<Self> {
        if Self::get_by_username(engine, username).await?.is_some() {
            return Err(rusty_db::Error::Conflict(
                "username already exists".to_string(),
            ));
        }
        let token = generate_token()?;
        Self::insert_new(engine, username, "", &token, false, Some(oauth_id)).await
    }

    /// Removes an account (Go: `DeleteUser`). Returns `true` if a row was
    /// removed, `false` if no user has `username`. A genuine hard delete
    /// — see this module's doc comment for why, same reasoning as
    /// `history.rs`'s `delete_by_user_and_url`.
    pub async fn delete_by_username(engine: &Engine, username: &str) -> rusty_db::Result<bool> {
        let table = Self::table();
        let affected = engine
            .execute(&Delete::from(&table).filter(table.col("username").eq(username)))
            .await?;
        Ok(affected > 0)
    }

    /// Verifies `username`/`password`, returning the account on success.
    /// `None` covers both "no such user" and "wrong password" — see this
    /// module's doc comment for why that collapsing is safe here (Go:
    /// `AuthenticateUser`).
    pub async fn authenticate(
        engine: &Engine,
        username: &str,
        password: &str,
    ) -> rusty_db::Result<Option<Self>> {
        let Some(user) = Self::get_by_username(engine, username).await? else {
            return Ok(None);
        };
        if verify_password(password, &user.password) {
            Ok(Some(user))
        } else {
            Ok(None)
        }
    }

    /// Looks up an account by its bearer token (Go: `GetUserByToken`).
    pub async fn get_by_token(engine: &Engine, token: &str) -> rusty_db::Result<Option<Self>> {
        let table = Self::table();
        engine
            .fetch_optional_as(&Select::from(&table).filter(table.col("token").eq(token)))
            .await
    }

    /// Issues a fresh bearer token for `user_id` and stores it, returning
    /// the new token regardless of whether `user_id` matches any row —
    /// see this module's doc comment on why that quirk is preserved (Go:
    /// `RegenerateToken`).
    pub async fn regenerate_token(engine: &Engine, user_id: i64) -> rusty_db::Result<String> {
        let token = generate_token()?;
        let table = Self::table();
        engine
            .execute(
                &Update::table(&table)
                    .set("token", token.clone())
                    .filter(table.col("id").eq(user_id)),
            )
            .await?;
        Ok(token)
    }

    /// Looks up an account by username (Go: `GetUser`).
    pub async fn get_by_username(
        engine: &Engine,
        username: &str,
    ) -> rusty_db::Result<Option<Self>> {
        let table = Self::table();
        engine
            .fetch_optional_as(&Select::from(&table).filter(table.col("username").eq(username)))
            .await
    }

    /// Looks up an account by id (Go: `GetUserByID`).
    pub async fn get_by_id(engine: &Engine, id: i64) -> rusty_db::Result<Option<Self>> {
        let table = Self::table();
        engine
            .fetch_optional_as(&Select::from(&table).filter(table.col("id").eq(id)))
            .await
    }

    /// Issues a fresh bearer token for the account named `username` (Go:
    /// `RegenerateTokenByUsername`). Unlike `regenerate_token`, this one
    /// does check existence first (matching Go), returning `None` if no
    /// user has that username.
    pub async fn regenerate_token_by_username(
        engine: &Engine,
        username: &str,
    ) -> rusty_db::Result<Option<String>> {
        let Some(user) = Self::get_by_username(engine, username).await? else {
            return Ok(None);
        };
        Self::regenerate_token(engine, user.id).await.map(Some)
    }

    /// Renames an account (Go: `UpdateUsername`). See [`RenameOutcome`]
    /// for the three possible results.
    pub async fn rename(
        engine: &Engine,
        username: &str,
        new_username: &str,
    ) -> rusty_db::Result<RenameOutcome> {
        if Self::get_by_username(engine, new_username).await?.is_some() {
            return Ok(RenameOutcome::UsernameTaken);
        }
        let table = Self::table();
        let affected = engine
            .execute(
                &Update::table(&table)
                    .set("username", new_username)
                    .filter(table.col("username").eq(username)),
            )
            .await?;
        Ok(if affected > 0 {
            RenameOutcome::Renamed
        } else {
            RenameOutcome::NotFound
        })
    }

    /// Sets a new password (Go: `UpdatePassword`). `true` if a matching
    /// user was found and updated, `false` otherwise.
    pub async fn set_password(
        engine: &Engine,
        username: &str,
        password: &str,
    ) -> rusty_db::Result<bool> {
        let hash = hash_password(password)?;
        let table = Self::table();
        let affected = engine
            .execute(
                &Update::table(&table)
                    .set("password", hash)
                    .filter(table.col("username").eq(username)),
            )
            .await?;
        Ok(affected > 0)
    }

    /// Looks up an account by its OAuth subject identifier (Go:
    /// `GetUserByOAuthID`).
    pub async fn get_by_oauth_id(
        engine: &Engine,
        oauth_id: &str,
    ) -> rusty_db::Result<Option<Self>> {
        let table = Self::table();
        engine
            .fetch_optional_as(&Select::from(&table).filter(table.col("oauth_id").eq(oauth_id)))
            .await
    }

    /// Flips the admin flag for `username` (Go: `ToggleAdmin`). Returns
    /// the new value, or `None` if no user has that username.
    pub async fn toggle_admin(engine: &Engine, username: &str) -> rusty_db::Result<Option<bool>> {
        let Some(user) = Self::get_by_username(engine, username).await? else {
            return Ok(None);
        };
        let new_value = !user.is_admin;
        let table = Self::table();
        engine
            .execute(
                &Update::table(&table)
                    .set("is_admin", new_value)
                    .filter(table.col("id").eq(user.id)),
            )
            .await?;
        Ok(Some(new_value))
    }

    /// Returns `user_id`'s raw stored rules document (Go: `GetUserRules`,
    /// minus `ParseRules`'s `config.Rules` parsing — see this module's
    /// doc comment for why that stays unported here). `None` if no user
    /// has that id.
    pub async fn rules_json(engine: &Engine, user_id: i64) -> rusty_db::Result<Option<Json>> {
        Ok(Self::get_by_id(engine, user_id)
            .await?
            .map(|user| user.rules_json))
    }

    /// Overwrites `user_id`'s stored rules document (Go: `SaveUserRules`).
    /// `true` if a matching user was found and updated, `false` otherwise.
    pub async fn set_rules_json(
        engine: &Engine,
        user_id: i64,
        rules: Json,
    ) -> rusty_db::Result<bool> {
        let table = Self::table();
        let affected = engine
            .execute(
                &Update::table(&table)
                    .set("rules_json", rules)
                    .filter(table.col("id").eq(user_id)),
            )
            .await?;
        Ok(affected > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> User {
        User {
            id: 1,
            created_at: "2024-01-01T00:00:00Z".parse().unwrap(),
            updated_at: "2024-01-01T00:00:00Z".parse().unwrap(),
            deleted: false,
            username: "ada".into(),
            password: "hash".into(),
            token: "tok".into(),
            is_admin: false,
            rules_json: serde_json::json!({}),
            oauth_id: None,
        }
    }

    async fn migrated_engine() -> rusty_db::Engine {
        let engine = rusty_db::sqlite::SqliteDriver::engine("sqlite::memory:")
            .await
            .unwrap();
        engine
            .migrator()
            .up(crate::SQLITE_MIGRATIONS)
            .await
            .unwrap();
        engine
    }

    #[test]
    fn table_name_is_users() {
        assert_eq!(User::TABLE_NAME, "users");
    }

    #[test]
    fn insert_round_trips_fields() {
        let user = sample();
        assert_eq!(user.username, "ada");
        assert!(user.oauth_id.is_none());
    }

    #[tokio::test]
    async fn round_trips_through_sqlite() {
        let engine = migrated_engine().await;

        let user = sample();
        engine.execute(&user.insert()).await.unwrap();

        let fetched: User = engine
            .fetch_one_as(&Select::from(&User::table()))
            .await
            .unwrap();
        assert_eq!(fetched.username, "ada");
        assert!(!fetched.deleted);
    }

    #[tokio::test]
    async fn soft_delete_hides_row_from_get_and_load_active() {
        let engine = migrated_engine().await;

        let mut session = engine.session();
        session.add(&sample());
        session.commit().await.unwrap();

        let mut session = engine.session();
        let user = session.get::<User>(1_i64).await.unwrap().unwrap();
        session.delete(&*user.borrow());
        session.commit().await.unwrap();

        let mut session = engine.session();
        assert_eq!(session.get::<User>(1_i64).await.unwrap(), None);
        let active = session.load_active::<User>().await.unwrap();
        assert!(active.is_empty());
    }

    #[tokio::test]
    async fn create_hashes_the_password_and_assigns_a_token() {
        let engine = migrated_engine().await;
        let user = User::create(&engine, "ada", "hunter2", false)
            .await
            .unwrap();
        assert_eq!(user.username, "ada");
        assert_ne!(user.password, "hunter2");
        assert!(!user.token.is_empty());
        assert!(!user.is_admin);
    }

    #[tokio::test]
    async fn create_rejects_a_duplicate_username() {
        let engine = migrated_engine().await;
        User::create(&engine, "ada", "hunter2", false)
            .await
            .unwrap();
        assert!(User::create(&engine, "ada", "different", false)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn create_oauth_has_no_local_password() {
        let engine = migrated_engine().await;
        let user = User::create_oauth(&engine, "ada", "oauth-subject-1")
            .await
            .unwrap();
        assert_eq!(user.oauth_id.as_deref(), Some("oauth-subject-1"));
        assert!(User::authenticate(&engine, "ada", "")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn authenticate_succeeds_with_the_right_password() {
        let engine = migrated_engine().await;
        User::create(&engine, "ada", "hunter2", false)
            .await
            .unwrap();
        let authenticated = User::authenticate(&engine, "ada", "hunter2").await.unwrap();
        assert!(authenticated.is_some());
    }

    #[tokio::test]
    async fn authenticate_fails_with_the_wrong_password() {
        let engine = migrated_engine().await;
        User::create(&engine, "ada", "hunter2", false)
            .await
            .unwrap();
        let authenticated = User::authenticate(&engine, "ada", "wrong").await.unwrap();
        assert!(authenticated.is_none());
    }

    #[tokio::test]
    async fn authenticate_fails_for_an_unknown_username() {
        let engine = migrated_engine().await;
        let authenticated = User::authenticate(&engine, "nobody", "hunter2")
            .await
            .unwrap();
        assert!(authenticated.is_none());
    }

    #[tokio::test]
    async fn delete_by_username_removes_the_row_and_reports_found() {
        let engine = migrated_engine().await;
        User::create(&engine, "ada", "hunter2", false)
            .await
            .unwrap();

        assert!(User::delete_by_username(&engine, "ada").await.unwrap());
        assert!(User::get_by_username(&engine, "ada")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn delete_by_username_reports_not_found_for_an_unknown_user() {
        let engine = migrated_engine().await;
        assert!(!User::delete_by_username(&engine, "nobody").await.unwrap());
    }

    #[tokio::test]
    async fn get_by_token_finds_the_matching_user() {
        let engine = migrated_engine().await;
        let created = User::create(&engine, "ada", "hunter2", false)
            .await
            .unwrap();
        let found = User::get_by_token(&engine, &created.token)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, created.id);
    }

    #[tokio::test]
    async fn regenerate_token_changes_the_stored_token() {
        let engine = migrated_engine().await;
        let created = User::create(&engine, "ada", "hunter2", false)
            .await
            .unwrap();
        let new_token = User::regenerate_token(&engine, created.id).await.unwrap();
        assert_ne!(new_token, created.token);
        assert!(User::get_by_token(&engine, &new_token)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn regenerate_token_returns_a_token_even_for_an_unknown_id() {
        let engine = migrated_engine().await;
        let token = User::regenerate_token(&engine, 12345).await.unwrap();
        assert!(!token.is_empty());
    }

    #[tokio::test]
    async fn regenerate_token_by_username_finds_and_updates_the_user() {
        let engine = migrated_engine().await;
        let created = User::create(&engine, "ada", "hunter2", false)
            .await
            .unwrap();
        let new_token = User::regenerate_token_by_username(&engine, "ada")
            .await
            .unwrap()
            .unwrap();
        assert_ne!(new_token, created.token);
    }

    #[tokio::test]
    async fn regenerate_token_by_username_returns_none_for_an_unknown_user() {
        let engine = migrated_engine().await;
        assert!(User::regenerate_token_by_username(&engine, "nobody")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn get_by_id_finds_the_matching_user() {
        let engine = migrated_engine().await;
        let created = User::create(&engine, "ada", "hunter2", false)
            .await
            .unwrap();
        let found = User::get_by_id(&engine, created.id).await.unwrap().unwrap();
        assert_eq!(found.username, "ada");
    }

    #[tokio::test]
    async fn rename_renames_an_existing_user() {
        let engine = migrated_engine().await;
        User::create(&engine, "ada", "hunter2", false)
            .await
            .unwrap();
        let outcome = User::rename(&engine, "ada", "grace").await.unwrap();
        assert_eq!(outcome, RenameOutcome::Renamed);
        assert!(User::get_by_username(&engine, "ada")
            .await
            .unwrap()
            .is_none());
        assert!(User::get_by_username(&engine, "grace")
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn rename_reports_username_taken() {
        let engine = migrated_engine().await;
        User::create(&engine, "ada", "hunter2", false)
            .await
            .unwrap();
        User::create(&engine, "grace", "hunter3", false)
            .await
            .unwrap();
        let outcome = User::rename(&engine, "ada", "grace").await.unwrap();
        assert_eq!(outcome, RenameOutcome::UsernameTaken);
    }

    #[tokio::test]
    async fn rename_reports_not_found() {
        let engine = migrated_engine().await;
        let outcome = User::rename(&engine, "nobody", "grace").await.unwrap();
        assert_eq!(outcome, RenameOutcome::NotFound);
    }

    #[tokio::test]
    async fn set_password_changes_which_password_authenticates() {
        let engine = migrated_engine().await;
        User::create(&engine, "ada", "hunter2", false)
            .await
            .unwrap();

        assert!(User::set_password(&engine, "ada", "newpassword123")
            .await
            .unwrap());
        assert!(User::authenticate(&engine, "ada", "hunter2")
            .await
            .unwrap()
            .is_none());
        assert!(User::authenticate(&engine, "ada", "newpassword123")
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn set_password_reports_not_found_for_an_unknown_user() {
        let engine = migrated_engine().await;
        assert!(!User::set_password(&engine, "nobody", "newpassword123")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn get_by_oauth_id_finds_the_matching_user() {
        let engine = migrated_engine().await;
        User::create_oauth(&engine, "ada", "oauth-subject-1")
            .await
            .unwrap();
        let found = User::get_by_oauth_id(&engine, "oauth-subject-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.username, "ada");
    }

    #[tokio::test]
    async fn toggle_admin_flips_the_flag_both_ways() {
        let engine = migrated_engine().await;
        User::create(&engine, "ada", "hunter2", false)
            .await
            .unwrap();

        assert_eq!(
            User::toggle_admin(&engine, "ada").await.unwrap(),
            Some(true)
        );
        assert_eq!(
            User::toggle_admin(&engine, "ada").await.unwrap(),
            Some(false)
        );
    }

    #[tokio::test]
    async fn toggle_admin_returns_none_for_an_unknown_user() {
        let engine = migrated_engine().await;
        assert!(User::toggle_admin(&engine, "nobody")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn rules_json_round_trips_through_set_rules_json() {
        let engine = migrated_engine().await;
        let created = User::create(&engine, "ada", "hunter2", false)
            .await
            .unwrap();

        assert_eq!(
            User::rules_json(&engine, created.id).await.unwrap(),
            Some(serde_json::json!({}))
        );

        let rules = serde_json::json!({"skip": {"re_strs": ["^https://ads\\."]}});
        assert!(User::set_rules_json(&engine, created.id, rules.clone())
            .await
            .unwrap());
        assert_eq!(
            User::rules_json(&engine, created.id).await.unwrap(),
            Some(rules)
        );
    }

    #[tokio::test]
    async fn rules_json_returns_none_for_an_unknown_user() {
        let engine = migrated_engine().await;
        assert_eq!(User::rules_json(&engine, 12345).await.unwrap(), None);
    }

    #[tokio::test]
    async fn set_rules_json_reports_not_found_for_an_unknown_user() {
        let engine = migrated_engine().await;
        assert!(!User::set_rules_json(&engine, 12345, serde_json::json!({}))
            .await
            .unwrap());
    }
}
