//! Test bulletin write
use shm_rs::bulletin::BulletinBoard;

fn main() {
    println!("Opening bulletin...");
    match BulletinBoard::open(None) {
        Ok(mut bulletin) => {
            println!("Opened OK");
            
            // Write a test vote
            let votes = vec![(999i64, "Test vote from test-write", 0u32, 3u32)];
            bulletin.set_votes(&votes);
            
            match bulletin.commit() {
                Ok(_) => println!("Commit SUCCESS"),
                Err(e) => println!("Commit FAILED: {}", e),
            }
        }
        Err(e) => println!("Open FAILED: {}", e),
    }
}
