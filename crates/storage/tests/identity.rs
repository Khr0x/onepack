use onepack_core::{Access, FeedName, PrincipalKind, PrincipalName, PublishPattern, Role};
use onepack_storage::{AuthFailure, AuthOutcome, Store, migrate};
use tempfile::TempDir;

async fn store() -> (Store, TempDir) {
    let dir = TempDir::new().unwrap();
    migrate(dir.path()).await.unwrap();
    (Store::open(dir.path()).await.unwrap(), dir)
}

async fn principal(store: &Store, name: &str, admin: bool) -> onepack_core::Principal {
    store
        .create_principal(
            &PrincipalName::parse(name).unwrap(),
            PrincipalKind::Service,
            admin,
            "test",
        )
        .await
        .unwrap()
}

fn valid(outcome: AuthOutcome) -> onepack_core::AuthContext {
    match outcome {
        AuthOutcome::Valid(ctx) => ctx,
        AuthOutcome::Invalid(f) => panic!("esperaba token válido: {f:?}"),
    }
}

fn invalid(outcome: AuthOutcome) -> AuthFailure {
    match outcome {
        AuthOutcome::Invalid(f) => f,
        AuthOutcome::Valid(_) => panic!("esperaba token inválido"),
    }
}

#[tokio::test]
async fn token_authenticates_with_grants() {
    let (store, _dir) = store().await;
    let feed = store
        .create_feed(&FeedName::parse("internal").unwrap(), "test")
        .await
        .unwrap();
    let ci = principal(&store, "ci-payments", false).await;
    let patterns = [PublishPattern::parse("Hemia.Payments.*").unwrap()];
    store
        .set_grant(&ci, &feed, Role::Publisher, &patterns, "test")
        .await
        .unwrap();
    let issued = store
        .create_token(&ci, Some("pipeline"), 3600, "test")
        .await
        .unwrap();

    let ctx = valid(store.authenticate(&issued.token).await.unwrap());
    assert_eq!(ctx.principal.name.as_str(), "ci-payments");
    assert_eq!(ctx.token_id, issued.id);
    assert_eq!(ctx.check_publish(feed.id, "hemia.payments.core"), Ok(()));
    assert!(ctx.check_publish(feed.id, "hemia.logging").is_err());
    assert!(ctx.check_feed(feed.id, Access::Maintain).is_err());
}

#[tokio::test]
async fn revoked_expired_and_wrong_tokens_fail() {
    let (store, _dir) = store().await;
    let p = principal(&store, "user", false).await;

    let revoked = store.create_token(&p, None, 3600, "test").await.unwrap();
    assert!(store.revoke_token(&revoked.id, "test").await.unwrap());
    assert!(
        !store.revoke_token(&revoked.id, "test").await.unwrap(),
        "revocar dos veces no hace nada"
    );
    assert_eq!(
        invalid(store.authenticate(&revoked.token).await.unwrap()),
        AuthFailure::Revoked {
            id: revoked.id.clone()
        }
    );

    let expired = store.create_token(&p, None, -1, "test").await.unwrap();
    assert_eq!(
        invalid(store.authenticate(&expired.token).await.unwrap()),
        AuthFailure::Expired {
            id: expired.id.clone()
        }
    );

    let good = store.create_token(&p, None, 3600, "test").await.unwrap();
    let tampered = format!("{}0", &good.token[..good.token.len() - 1]);
    let tampered = if tampered == good.token {
        format!("{}1", &good.token[..good.token.len() - 1])
    } else {
        tampered
    };
    assert_eq!(
        invalid(store.authenticate(&tampered).await.unwrap()),
        AuthFailure::WrongSecret {
            id: good.id.clone()
        }
    );

    let unknown = format!("opk_{}_{}", "0".repeat(16), "0".repeat(64));
    assert!(matches!(
        invalid(store.authenticate(&unknown).await.unwrap()),
        AuthFailure::UnknownToken { .. }
    ));
    assert_eq!(
        invalid(store.authenticate("not-a-token").await.unwrap()),
        AuthFailure::Malformed
    );

    // Los demás tokens del principal siguen siendo válidos.
    valid(store.authenticate(&good.token).await.unwrap());
}

#[tokio::test]
async fn grants_can_be_replaced_and_removed() {
    let (store, _dir) = store().await;
    let feed = store
        .create_feed(&FeedName::parse("internal").unwrap(), "test")
        .await
        .unwrap();
    let p = principal(&store, "dev", false).await;
    let token = store
        .create_token(&p, None, 3600, "test")
        .await
        .unwrap()
        .token;

    store
        .set_grant(
            &p,
            &feed,
            Role::Publisher,
            &[PublishPattern::parse("A.*").unwrap()],
            "test",
        )
        .await
        .unwrap();
    store
        .set_grant(&p, &feed, Role::Reader, &[], "test")
        .await
        .unwrap();
    let grants = store.grants(&p).await.unwrap();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].role, Role::Reader);
    assert!(
        grants[0].publish_patterns.is_empty(),
        "el reemplazo borra los patrones anteriores"
    );

    assert!(store.remove_grant(&p, &feed, "test").await.unwrap());
    let ctx = valid(store.authenticate(&token).await.unwrap());
    assert!(ctx.grants.is_empty());
}

#[tokio::test]
async fn database_never_contains_the_secret() {
    let (store, dir) = store().await;
    let p = principal(&store, "admin", true).await;
    let issued = store.create_token(&p, None, 3600, "test").await.unwrap();
    store.close().await;

    let secret = issued.token.rsplit('_').next().unwrap();
    for entry in std::fs::read_dir(dir.path()).unwrap().flatten() {
        if entry.path().is_file() {
            let bytes = std::fs::read(entry.path()).unwrap();
            assert!(
                !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
                "{} contiene el secreto",
                entry.path().display()
            );
        }
    }
}

#[tokio::test]
async fn mutations_are_audited() {
    let (store, dir) = store().await;
    let feed = store
        .create_feed(&FeedName::parse("internal").unwrap(), "admin")
        .await
        .unwrap();
    let p = principal(&store, "dev", false).await;
    store
        .set_grant(&p, &feed, Role::Reader, &[], "admin")
        .await
        .unwrap();
    let t = store.create_token(&p, None, 3600, "admin").await.unwrap();
    store.revoke_token(&t.id, "admin").await.unwrap();
    store.remove_grant(&p, &feed, "admin").await.unwrap();
    store
        .record_auth_failure(&AuthFailure::WrongSecret { id: t.id.clone() })
        .await;
    store.close().await;

    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        dir.path().join("metadata.sqlite").display()
    ))
    .await
    .unwrap();
    let actions: Vec<String> = sqlx::query_scalar("SELECT action FROM audit_event ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(
        actions,
        [
            "feed.create",
            "principal.create",
            "grant.set",
            "token.create",
            "token.revoke",
            "grant.remove",
            "auth.failure"
        ]
    );
}
