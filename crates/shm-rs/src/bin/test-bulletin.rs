use shm_rs::bulletin::BulletinBoard;

fn main() {
    println!("Opening bulletin...");
    match BulletinBoard::open(None) {
        Ok(mut bulletin) => {
            println!("Opened OK");
            
            // Simulate what the daemon does
            let vote_data: Vec<(i64, &str, u32, u32)> = vec![
                (4, "Fresh vote after restart", 0, 0),
                (3, "Should we use BulletinBoard?", 0, 0),
            ];
            
            bulletin.set_votes(&vote_data);
            bulletin.commit().expect("commit failed");
            
            println!("Wrote {} votes", vote_data.len());
            println!("Hook output:\n{}", bulletin.to_hook_output());
        }
        Err(e) => println!("Failed to open: {}", e),
    }
}
