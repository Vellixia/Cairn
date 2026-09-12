//! Explicit knowledge mutations become commands once the server owns knowledge
//! (T027, FR-701, FR-712, FR-815a).
//!
//! Before cutover a `cairn remember` is a local durable record that syncs
//! afterwards. After cutover the same call must become a **request** — FR-712
//! forbids "a local write the server later discovers" — and the caller is told
//! the command was accepted for delivery, never that it is durable.
//!
//! Two things this asserts that are easy to get wrong in opposite directions:
//! a `local_only` memory stays local, because knowledge the user asked never to
//! leave the machine is the one case routing it to the server would invert; and
//! a command with no account is refused rather than queued, because the claim
//! predicate matches an account exactly, so an accountless row is a black hole
//! rather than a queued write.

use cairn_e2e::feature005::Local;
use cairn_store::authority::{self, AuthorityMode};
use cairn_store::spool;
use uuid::Uuid;

/// The server instance every fixture in this file queues for and drains as.
///
/// A constant rather than a fresh id per test, because the property under test
/// here is capacity and ordering, not identity — the instance binding has its
/// own file (`feature005_spool_instance_binding.rs`), and it is the one place
/// that varies this value. Fixing it here keeps every claim addressing the same
/// deployment, which is what these tests were written assuming before the
/// binding existed.
const FIXTURE_INSTANCE: uuid::Uuid =
    uuid::Uuid::from_u128(0x0005_0004_0000_0000_0000_0000_0000_0001);

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

#[test]
fn a_store_starts_before_cutover_and_writes_locally() {
    rt().block_on(async {
        let db = Local::new().await;
        assert_eq!(
            authority::mode(&db.store).await.unwrap(),
            AuthorityMode::Feature004
        );
        assert!(
            !authority::mode(&db.store)
                .await
                .unwrap()
                .commands_are_authoritative(),
            "a fresh store would have routed its first write to a server it has \
             never spoken to"
        );
    });
}

#[test]
fn migration_does_not_flip_the_routing_half_way_through() {
    rt().block_on(async {
        // Migration moves what already exists. Turning new writes into commands
        // while it runs would leave a store whose recent knowledge went one way
        // and whose older knowledge went another.
        let db = Local::new().await;
        authority::set_mode(&db.store, AuthorityMode::Migrating)
            .await
            .unwrap();
        assert!(!authority::mode(&db.store)
            .await
            .unwrap()
            .commands_are_authoritative());
    });
}

#[test]
fn after_cutover_a_command_is_queued_account_bound_and_in_scope_order() {
    rt().block_on(async {
        let db = Local::new().await;
        authority::set_mode(&db.store, AuthorityMode::Migrating)
            .await
            .unwrap();
        authority::set_mode(&db.store, AuthorityMode::ServerAuthoritative)
            .await
            .unwrap();

        let account = Uuid::now_v7();
        let payload = serde_json::json!({ "content": "an intent" });
        let scope = spool::store_scope(&db.store).await.expect("store scope");

        // Sessionless, store-scoped: the CLI permits memory operations outside
        // any session, and a throwaway session row would leave a second active
        // session in the worktree.
        for (n, kind) in [spool::CommandKind::Remember, spool::CommandKind::Supersede]
            .into_iter()
            .enumerate()
        {
            let admission = spool::spool_command(
                &db.store,
                spool::NewCommand {
                    scope,
                    project_id: None,
                    account_id: account,
                    // Unbound: these fixtures predate any established instance, which is
                    // the state the first-binding rule adopts on the first claim.
                    server_instance_id: None,
                    kind,
                    payload: &payload,
                },
                spool::SpoolCapacity::default(),
            )
            .await
            .expect("spool");
            match admission {
                spool::CommandAdmission::Spooled(c) => {
                    assert_eq!(c.command_seq, n as u64 + 1, "scope ordering was not kept");
                    assert_eq!(c.session_id, None, "a session was invented");
                    assert_eq!(c.project_id, None);
                }
                other => panic!("the command was not queued: {other:?}"),
            }
        }

        // Account-bound: another account's drain claims none of it.
        assert!(
            spool::claim_commands(&db.store, Uuid::now_v7(), FIXTURE_INSTANCE, 10)
                .await
                .expect("claim")
                .is_empty()
        );
        assert_eq!(
            spool::claim_commands(&db.store, account, FIXTURE_INSTANCE, 10)
                .await
                .expect("claim")
                .len(),
            2
        );
    });
}

#[test]
fn nothing_local_becomes_authoritative_because_a_command_is_waiting() {
    rt().block_on(async {
        // FR-709 and FR-787. A queued command is not a local durable record,
        // and the store must hold no knowledge row as a side effect of one
        // being queued.
        let db = Local::new().await;
        authority::set_mode(&db.store, AuthorityMode::Migrating)
            .await
            .unwrap();
        authority::set_mode(&db.store, AuthorityMode::ServerAuthoritative)
            .await
            .unwrap();

        let payload = serde_json::json!({ "content": "an intent" });
        spool::spool_command(
            &db.store,
            spool::NewCommand {
                scope: spool::store_scope(&db.store).await.unwrap(),
                project_id: None,
                account_id: Uuid::now_v7(),
                // Unbound: these fixtures predate any established instance, which is
                // the state the first-binding rule adopts on the first claim.
                server_instance_id: None,
                kind: spool::CommandKind::Remember,
                payload: &payload,
            },
            spool::SpoolCapacity::default(),
        )
        .await
        .expect("spool");

        assert_eq!(db.count("SELECT count(*) FROM memories").await, 0);
        assert_eq!(db.count("SELECT count(*) FROM personal_knowledge").await, 0);
        assert_eq!(db.count("SELECT count(*) FROM command_spool").await, 1);
    });
}
