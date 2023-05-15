use std::io;

fn main() -> io::Result<()> {
    tracing_subscriber::fmt::init();
    let mut args = std::env::args();
    let _progname = args.next().unwrap();
    let callsign = args.next().unwrap();

    let ctydat = ctydat::Ctydat::from_path("cty.dat")?;

    if let Some(country) = ctydat.search_callsign(&callsign) {
        println!("{:#?}", country);
    }

    Ok(())
}
