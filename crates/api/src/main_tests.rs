use super::*;

#[test]
fn startup_returns_failure_when_the_http_address_cannot_be_bound() {
    let config = Config::load().expect("repository configuration is valid");
    let listener = std::net::TcpListener::bind(config.server.bind)
        .expect("reserve the configured HTTP address");

    let result = main();

    assert_eq!(result, ExitCode::FAILURE);
    drop(listener);
}
