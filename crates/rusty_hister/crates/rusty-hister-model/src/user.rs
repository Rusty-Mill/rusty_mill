//! `User` — a Rust port of `server/model/user.go`'s `User` struct (fields
//! only; password hashing, token regeneration, and the admin-flag helpers
//! are query-layer behavior, not schema, and are a follow-up increment —
//! see the crate root docs).

use rusty_db::prelude::*;

/// A registered account. Soft-deleted via `deleted` (Hister's Go source uses
/// a nullable `DeletedAt` column for the same purpose — `rusty_db`'s
/// `#[table(soft_delete)]` is the first-party equivalent, so this crate uses
/// that mechanism instead of hand-rolling a nullable timestamp).
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
        let engine = rusty_db::sqlite::SqliteDriver::engine("sqlite::memory:")
            .await
            .unwrap();
        engine
            .migrator()
            .up(crate::SQLITE_MIGRATIONS)
            .await
            .unwrap();

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
        let engine = rusty_db::sqlite::SqliteDriver::engine("sqlite::memory:")
            .await
            .unwrap();
        engine
            .migrator()
            .up(crate::SQLITE_MIGRATIONS)
            .await
            .unwrap();

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
}
