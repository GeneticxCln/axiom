fn main() -> anyhow::Result<()> {
    env_logger::init();
    let server = axiom::experimental::smithay::MinimalServer::new()?;
    server.run()
}
