use sqlite_doctor::initial_hazards;

fn main() {
    println!("sqlite-doctor");
    println!("SQLite migration safety linter for ORM-generated rebuild hazards");
    println!();
    println!("Initial MVP hazard set:");

    for hazard in initial_hazards() {
        println!("- {hazard}");
    }
}
