//! Board members (P25 Stage 1).
//!
//! The human, every agent and the per-workspace Task Management Agent are one
//! entity. That is the whole point: a card's `assignee` is a member id and the
//! board renders a row the same way whoever is behind it, so "assign this to an
//! agent" and "assign this to a person" stop being different code paths.
//!
//! Like [`crate::tasks`], this module uses no SQL triggers or transactions —
//! every method holds the single connection mutex for the duration of its
//! writes, which is what makes a read-then-write invariant hold.

use crate::{Member, MemberKind, MemberOwner, MemberPatch, MemberStatus, NewMember, StorageError};
use std::sync::Arc;
use tokio::sync::Mutex;
use turso::Connection;

/// Id of the seeded human member.
///
/// Fixed rather than generated so `assignee = 'human'` resolves from the moment
/// the schema exists — a clarification card has somewhere to go before anyone
/// has configured anything.
pub const HUMAN_MEMBER_ID: &str = "human";

/// Prefix of every Task Management Agent's id; the suffix is the workspace id,
/// or [`ROUTER_GLOBAL_MEMBER_ID`]'s `global` for cards outside any workspace.
pub const ROUTER_MEMBER_PREFIX: &str = "router:";

/// Id of the router that owns globally-scoped cards.
pub const ROUTER_GLOBAL_MEMBER_ID: &str = "router:global";

/// Maximum member id length in bytes.
///
/// Bound justification: the id is stored in `tasks.assignee` and named on every
/// prompt line that says who a card belongs to. 128 bytes holds
/// `router:<workspace-uuid>` with room to spare and keeps an id from ever being
/// a meaningful share of a small model's window.
pub const MEMBER_ID_MAX_BYTES: usize = 128;

/// Maximum member name length in bytes.
///
/// Bound justification: the name is rendered on every card the member is
/// assigned to and injected once per member into the router's decision prompt.
/// 200 bytes (~50 tokens) is generous for a display name and bounds the roster
/// block at [`MEMBERS_MAX`] × 200 B.
pub const MEMBER_NAME_MAX_BYTES: usize = 200;

/// Maximum avatar reference length in bytes.
///
/// Bound justification: an avatar is a *reference* — a URL, an emoji, a short
/// token — never image bytes. 512 bytes admits every URL worth having and makes
/// the "do not store the image here" rule enforceable rather than advisory; a
/// data URI large enough to hold a picture cannot fit.
pub const MEMBER_AVATAR_MAX_BYTES: usize = 512;

/// Maximum serialized profile length in bytes.
///
/// Bound justification: the profile (model tier, capability tags, tools,
/// skills, cost) is read into the router's prompt on every routing decision.
/// 8 KiB is ~2k tokens — the most a single member's profile could usefully
/// contribute before it crowds out the card it is meant to help place.
pub const MEMBER_PROFILE_MAX_BYTES: usize = 8 * 1024;

/// Maximum members per install.
///
/// Bound justification: the router loads the whole roster to pick an assignee,
/// so the decision prompt costs `MEMBERS_MAX` × ([`MEMBER_PROFILE_MAX_BYTES`] +
/// name) ≈ 8 MiB worst case in memory and is the reason a roster cannot grow
/// without a ceiling. 1000 is far past a real board and brakes a runaway agent
/// creating members in a loop.
pub const MEMBERS_MAX: usize = 1000;

const MEMBER_COLUMNS: &str =
    "id, name, avatar, kind, owner_kind, owner_id, status, profile, created_at, updated_at";

/// The Task Management Agent's id for a given scope.
///
/// `None` means the global board, which shares [`ROUTER_GLOBAL_MEMBER_ID`].
#[must_use]
pub fn router_member_id(workspace_id: Option<&str>) -> String {
    workspace_id.map_or_else(
        || ROUTER_GLOBAL_MEMBER_ID.to_string(),
        |id| format!("{ROUTER_MEMBER_PREFIX}{id}"),
    )
}

/// Members repository over the shared Turso connection.
pub struct MemberRepository {
    conn: Arc<Mutex<Connection>>,
}

impl MemberRepository {
    #[must_use]
    pub const fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Create a member.
    ///
    /// # Errors
    /// [`StorageError::Invalid`] when a bound is exceeded, when the id is
    /// already taken, or when the roster is full; [`StorageError::Database`] if
    /// the insert fails.
    ///
    /// # Panics
    /// Never in practice: the debug assertion restates the id bound
    /// `validate_member` has already enforced on the line above.
    pub async fn create(&self, new: NewMember) -> Result<Member, StorageError> {
        let profile = serde_json::to_string(&new.profile)?;
        validate_member(&new.id, &new.name, new.avatar.as_deref(), &profile)?;
        debug_assert!(
            !new.id.is_empty() && new.id.len() <= MEMBER_ID_MAX_BYTES,
            "validate_member admitted an out-of-range id"
        );

        let conn = self.conn.lock().await;
        let count = count_members(&conn).await?;
        if count >= MEMBERS_MAX {
            drop(conn);
            return Err(StorageError::Invalid(format!(
                "member limit reached ({MEMBERS_MAX}); delete a member before adding another"
            )));
        }
        if find_member(&conn, &new.id).await?.is_some() {
            drop(conn);
            return Err(StorageError::Invalid(format!(
                "member '{}' already exists",
                new.id
            )));
        }

        let (id, name) = (new.id.clone(), new.name.clone());
        let avatar = new.avatar.clone();
        let (kind, owner_kind) = (new.kind.as_str(), new.owner_kind.as_str());
        let owner_id = new.owner_id.clone();
        let status = new.status.as_str();
        conn.execute(
            "INSERT INTO members (id, name, avatar, kind, owner_kind, owner_id, status, profile)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            turso::params![
                id, name, avatar, kind, owner_kind, owner_id, status, profile
            ],
        )
        .await?;

        let created = find_member(&conn, &new.id).await?;
        drop(conn);
        created.ok_or_else(|| StorageError::NotFound(format!("member '{}' after insert", new.id)))
    }

    /// Fetch one member.
    ///
    /// # Errors
    /// [`StorageError::NotFound`] when no such member exists;
    /// [`StorageError::Database`] if the query fails or a column does not decode.
    pub async fn get(&self, id: &str) -> Result<Member, StorageError> {
        let conn = self.conn.lock().await;
        let found = find_member(&conn, id).await?;
        drop(conn);
        found.ok_or_else(|| StorageError::NotFound(format!("member '{id}'")))
    }

    /// Whether a member id resolves.
    ///
    /// This is what makes `tasks.assignee` a real reference: SQLite cannot add
    /// a foreign key to an existing column and `PRAGMA foreign_keys` is off, so
    /// the check lives here and runs on write.
    ///
    /// # Errors
    /// [`StorageError::Database`] if the query fails.
    pub async fn exists(&self, id: &str) -> Result<bool, StorageError> {
        let conn = self.conn.lock().await;
        let found = find_member(&conn, id).await?;
        drop(conn);
        Ok(found.is_some())
    }

    /// Every member, oldest first.
    ///
    /// # Errors
    /// [`StorageError::Database`] if the query fails or a row does not decode.
    pub async fn list(&self) -> Result<Vec<Member>, StorageError> {
        let conn = self.conn.lock().await;
        let mut rows = conn
            .query(
                &format!("SELECT {MEMBER_COLUMNS} FROM members ORDER BY created_at ASC, id ASC"),
                turso::params![],
            )
            .await?;
        let mut members = Vec::new();
        while let Some(row) = rows.next().await? {
            members.push(row_to_member(&row)?);
        }
        // Held until the cursor is gone: an open `Rows` on the shared
        // connection swallows later writes.
        drop(rows);
        drop(conn);
        Ok(members)
    }

    /// The roster one board sees: members owned by this workspace, plus every
    /// human-owned member (the human and their personal agents, which travel
    /// with them between workspaces — P25 decision 13).
    ///
    /// `None` is the global board, whose workspace-owned members are the rows
    /// with a null `owner_id`.
    ///
    /// # Errors
    /// [`StorageError::Database`] if the query fails or a row does not decode.
    pub async fn list_for_workspace(
        &self,
        workspace_id: Option<&str>,
    ) -> Result<Vec<Member>, StorageError> {
        let conn = self.conn.lock().await;
        let owned = workspace_id.map(ToString::to_string);
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {MEMBER_COLUMNS} FROM members
                     WHERE owner_kind = 'human'
                        OR (owner_kind = 'workspace' AND owner_id IS ?1)
                     ORDER BY created_at ASC, id ASC"
                ),
                turso::params![owned],
            )
            .await?;
        let mut members = Vec::new();
        while let Some(row) = rows.next().await? {
            members.push(row_to_member(&row)?);
        }
        // Held until the cursor is gone: an open `Rows` on the shared
        // connection swallows later writes.
        drop(rows);
        drop(conn);
        Ok(members)
    }

    /// Apply a partial update. `None` fields are left untouched.
    ///
    /// # Errors
    /// [`StorageError::NotFound`] when the member does not exist,
    /// [`StorageError::Invalid`] when a bound is exceeded,
    /// [`StorageError::Database`] if a write fails.
    pub async fn update(&self, id: &str, patch: MemberPatch) -> Result<Member, StorageError> {
        let conn = self.conn.lock().await;
        let Some(current) = find_member(&conn, id).await? else {
            drop(conn);
            return Err(StorageError::NotFound(format!("member '{id}'")));
        };

        let name = patch.name.unwrap_or(current.name);
        let avatar = patch.avatar.unwrap_or(current.avatar);
        let owner_id = patch.owner_id.unwrap_or(current.owner_id);
        let status = patch.status.unwrap_or(current.status);
        let profile = serde_json::to_string(&patch.profile.unwrap_or(current.profile))?;
        if let Err(err) = validate_member(id, &name, avatar.as_deref(), &profile) {
            drop(conn);
            return Err(err);
        }

        let status_token = status.as_str();
        let target = id.to_string();
        conn.execute(
            "UPDATE members
                SET name = ?1, avatar = ?2, owner_id = ?3, status = ?4, profile = ?5,
                    updated_at = datetime('now')
              WHERE id = ?6",
            turso::params![name, avatar, owner_id, status_token, profile, target],
        )
        .await?;

        let updated = find_member(&conn, id).await?;
        drop(conn);
        updated.ok_or_else(|| StorageError::NotFound(format!("member '{id}' after update")))
    }

    /// Set a member's status — the only thing the board shows about
    /// availability, and the hot path (it moves on every run start and end).
    ///
    /// # Errors
    /// [`StorageError::NotFound`] when the member does not exist;
    /// [`StorageError::Database`] if the update fails.
    pub async fn set_status(&self, id: &str, status: MemberStatus) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        let (owned_id, token) = (id.to_string(), status.as_str());
        let affected = conn
            .execute(
                "UPDATE members SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
                turso::params![token, owned_id],
            )
            .await?;
        drop(conn);
        if affected == 0 {
            return Err(StorageError::NotFound(format!("member '{id}'")));
        }
        Ok(())
    }

    /// Delete a member.
    ///
    /// The seeded human and the routers are refused: a board with no human has
    /// nowhere to send a clarification, and a board with no router has nobody to
    /// place a card. Cards already assigned to a deleted member keep their
    /// `assignee` — the id stops resolving, which the router reads as an
    /// unassigned card rather than silently re-attributing work.
    ///
    /// # Errors
    /// [`StorageError::Invalid`] for a protected member;
    /// [`StorageError::Database`] if the delete fails.
    pub async fn delete(&self, id: &str) -> Result<bool, StorageError> {
        if is_protected_member(id) {
            return Err(StorageError::Invalid(format!(
                "member '{id}' is required by the board and cannot be deleted"
            )));
        }
        let conn = self.conn.lock().await;
        let owned_id = id.to_string();
        let affected = conn
            .execute(
                "DELETE FROM members WHERE id = ?1",
                turso::params![owned_id],
            )
            .await?;
        drop(conn);
        Ok(affected > 0)
    }

    /// Get the Task Management Agent for a board, creating it if the workspace
    /// was registered after the members migration ran.
    ///
    /// # Errors
    /// [`StorageError::Invalid`] when the roster is full;
    /// [`StorageError::Database`] if a query or the insert fails.
    pub async fn ensure_router(&self, workspace_id: Option<&str>) -> Result<Member, StorageError> {
        let id = router_member_id(workspace_id);
        {
            let conn = self.conn.lock().await;
            let existing = find_member(&conn, &id).await?;
            drop(conn);
            if let Some(member) = existing {
                return Ok(member);
            }
        }
        self.create(NewMember {
            id,
            name: "Task Router".to_string(),
            avatar: None,
            kind: MemberKind::Agent,
            owner_kind: MemberOwner::Workspace,
            owner_id: workspace_id.map(ToString::to_string),
            status: MemberStatus::Idle,
            profile: serde_json::json!({ "role": "router" }),
        })
        .await
    }

    /// How many members exist.
    ///
    /// # Errors
    /// [`StorageError::Database`] if the query fails.
    pub async fn count(&self) -> Result<usize, StorageError> {
        let conn = self.conn.lock().await;
        let count = count_members(&conn).await?;
        drop(conn);
        Ok(count)
    }
}

/// The members the board cannot function without.
fn is_protected_member(id: &str) -> bool {
    id == HUMAN_MEMBER_ID || id.starts_with(ROUTER_MEMBER_PREFIX)
}

fn validate_member(
    id: &str,
    name: &str,
    avatar: Option<&str>,
    profile_json: &str,
) -> Result<(), StorageError> {
    if id.is_empty() {
        return Err(StorageError::Invalid("member id must not be empty".into()));
    }
    if id.len() > MEMBER_ID_MAX_BYTES {
        return Err(StorageError::Invalid(format!(
            "member id is {} bytes, over the {MEMBER_ID_MAX_BYTES}-byte limit",
            id.len()
        )));
    }
    if name.trim().is_empty() {
        return Err(StorageError::Invalid(
            "member name must not be empty".into(),
        ));
    }
    if name.len() > MEMBER_NAME_MAX_BYTES {
        return Err(StorageError::Invalid(format!(
            "member name is {} bytes, over the {MEMBER_NAME_MAX_BYTES}-byte limit",
            name.len()
        )));
    }
    if let Some(avatar) = avatar
        && avatar.len() > MEMBER_AVATAR_MAX_BYTES
    {
        return Err(StorageError::Invalid(format!(
            "member avatar is {} bytes, over the {MEMBER_AVATAR_MAX_BYTES}-byte limit — an avatar \
             is a reference (URL, emoji, token), not image data",
            avatar.len()
        )));
    }
    if profile_json.len() > MEMBER_PROFILE_MAX_BYTES {
        return Err(StorageError::Invalid(format!(
            "member profile is {} bytes, over the {MEMBER_PROFILE_MAX_BYTES}-byte limit",
            profile_json.len()
        )));
    }
    Ok(())
}

async fn count_members(conn: &Connection) -> Result<usize, StorageError> {
    let mut rows = conn
        .query("SELECT COUNT(*) FROM members", turso::params![])
        .await?;
    let count = match rows.next().await? {
        Some(row) => row.get::<i64>(0)?,
        None => 0,
    };
    // Held until the cursor is gone: an open `Rows` on the shared connection
    // swallows later writes.
    drop(rows);
    usize::try_from(count).map_err(|_| {
        StorageError::Invalid(format!(
            "members COUNT(*) returned a negative value: {count}"
        ))
    })
}

async fn find_member(conn: &Connection, id: &str) -> Result<Option<Member>, StorageError> {
    let owned = id.to_string();
    let mut rows = conn
        .query(
            &format!("SELECT {MEMBER_COLUMNS} FROM members WHERE id = ?1"),
            turso::params![owned],
        )
        .await?;
    let member = match rows.next().await? {
        Some(row) => Some(row_to_member(&row)?),
        None => None,
    };
    // Held until the cursor is gone: an open `Rows` on the shared connection
    // swallows later writes.
    drop(rows);
    Ok(member)
}

fn row_to_member(row: &turso::Row) -> Result<Member, StorageError> {
    let id: String = row.get(0)?;
    let kind_token: String = row.get(3)?;
    let owner_token: String = row.get(4)?;
    let status_token: String = row.get(6)?;
    let profile_json: String = row.get(7)?;

    // A token this version does not know is a schema from the future, not a
    // default to fall back on — coercing it would silently reassign what a
    // member is.
    let kind = MemberKind::parse(&kind_token).ok_or_else(|| {
        StorageError::Invalid(format!("member '{id}' has unknown kind '{kind_token}'"))
    })?;
    let owner_kind = MemberOwner::parse(&owner_token).ok_or_else(|| {
        StorageError::Invalid(format!(
            "member '{id}' has unknown owner_kind '{owner_token}'"
        ))
    })?;
    let status = MemberStatus::parse(&status_token).ok_or_else(|| {
        StorageError::Invalid(format!("member '{id}' has unknown status '{status_token}'"))
    })?;

    Ok(Member {
        id,
        name: row.get(1)?,
        avatar: row.get(2)?,
        kind,
        owner_kind,
        owner_id: row.get(5)?,
        status,
        profile: serde_json::from_str(&profile_json)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}
