use super::*;

#[test]
fn identity_insert_mapper_preserves_non_unique_database_errors() {
    assert!(matches!(
        first_sign_in_identity_error(sqlx::Error::RowNotFound),
        AppError::Database(sqlx::Error::RowNotFound)
    ));
}
