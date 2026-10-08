use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test]
async fn photos_cannot_be_inserted_without_a_voyage(pool: PgPool) {
    let owner = pinvoyager_api::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    let result = sqlx::query("INSERT INTO photos (id, owner_id, content_hash, byte_size, width, height, original_format) VALUES ($1, $2, $3, 1, 1, 1, 'jpeg')")
        .bind(Uuid::now_v7()).bind(owner.id).bind("a".repeat(64)).execute(&pool).await;
    assert!(
        result.is_err(),
        "the database must reject an unattached photo"
    );
}

#[sqlx::test(migrations = false)]
async fn migration_preserves_library_and_shared_photos(pool: PgPool) {
    for migration in sqlx::migrate!("./migrations")
        .iter()
        .filter(|migration| migration.version < 20261006000001)
    {
        sqlx::raw_sql(sqlx::AssertSqlSafe(migration.sql.as_ref()))
            .execute(&pool)
            .await
            .expect("legacy migration");
    }
    let owner = Uuid::now_v7();
    let first = Uuid::now_v7();
    let second = Uuid::now_v7();
    let shared = Uuid::now_v7();
    let library = Uuid::now_v7();
    sqlx::query("INSERT INTO users (id) VALUES ($1)")
        .bind(owner)
        .execute(&pool)
        .await
        .expect("owner");
    for (id, title) in [(first, "First"), (second, "Second")] {
        sqlx::query("INSERT INTO voyages (id, owner_id, title) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(owner)
            .bind(title)
            .execute(&pool)
            .await
            .expect("voyage");
    }
    for id in [shared, library] {
        sqlx::query("INSERT INTO photos (id, owner_id, content_hash, byte_size, width, height, original_format) VALUES ($1, $2, $3, 1, 1, 1, 'jpeg')")
            .bind(id).bind(owner).bind("a".repeat(64)).execute(&pool).await.expect("legacy photo");
    }
    for voyage in [first, second] {
        sqlx::query("INSERT INTO voyage_photos (voyage_id, photo_id, position) VALUES ($1, $2, 1)")
            .bind(voyage)
            .bind(shared)
            .execute(&pool)
            .await
            .expect("membership");
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("migrations/20261006000001_photos_belong_to_voyages.sql");
    let migration = std::fs::read_to_string(path).expect("voyage ownership migration exists");
    sqlx::raw_sql(sqlx::AssertSqlSafe(migration.as_str()))
        .execute(&pool)
        .await
        .expect("new migration");
    let orphaned: i64 = sqlx::query_scalar("SELECT count(*) FROM photos WHERE voyage_id IS NULL")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(orphaned, 0);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM photos")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(
        count, 3,
        "shared membership becomes two photo records; the library photo survives"
    );
    for voyage in [first, second] {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM voyage_photos WHERE voyage_id = $1")
                .bind(voyage)
                .fetch_one(&pool)
                .await
                .expect("count");
        assert_eq!(count, 1);
    }
    let recovered: (String, bool) = sqlx::query_as("SELECT v.title, v.published FROM voyages v JOIN photos p ON p.voyage_id = v.id WHERE p.id = $1").bind(library).fetch_one(&pool).await.expect("recovered draft");
    assert_eq!(recovered, ("Recovered photos".into(), false));
}

#[sqlx::test]
async fn a_photo_cannot_name_another_owners_voyage(pool: PgPool) {
    let owner = pinvoyager_api::db::repo::users::create(&pool, None, None)
        .await
        .expect("owner");
    let stranger = pinvoyager_api::db::repo::users::create(&pool, None, None)
        .await
        .expect("stranger");
    let voyage: Uuid = sqlx::query_scalar(
        "INSERT INTO voyages(id, owner_id, title) VALUES($1, $2, 'Private') RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(owner.id)
    .fetch_one(&pool)
    .await
    .expect("voyage");
    let result = sqlx::query("INSERT INTO photos(id, owner_id, voyage_id, content_hash, byte_size, width, height, original_format) VALUES($1, $2, $3, $4, 1, 1, 1, 'jpeg')")
        .bind(Uuid::now_v7()).bind(stranger.id).bind(voyage).bind("b".repeat(64)).execute(&pool).await;
    assert!(result.is_err(), "voyage and photo owners must match");
}
