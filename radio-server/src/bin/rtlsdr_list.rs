use rigflow_server::source::rtlsdr::RtlSdrSource;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", RtlSdrSource::device_summary()?);
    Ok(())
}
