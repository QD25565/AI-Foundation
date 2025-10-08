// Matrix-style falling code effect with depth layers
// Code snippets are loaded from codeSnippets.js (loaded first)
function initMatrixEffect() {
    const dataStream = document.getElementById('dataStream');
    const columns = Math.floor(window.innerWidth / 80) * 3;
    
    // Create FOREGROUND layer (60% of columns) - normal speed, normal size
    const foregroundCount = Math.floor(columns * 0.6);
    for (let i = 0; i < foregroundCount; i++) {
        createColumn(i, 'foreground');
    }
    
    // Create BACKGROUND layer (40% of columns) - slower, smaller, more transparent
    const backgroundCount = Math.floor(columns * 0.4);
    for (let i = 0; i < backgroundCount; i++) {
        createColumn(i, 'background');
    }
    
    function createColumn(index, layer) {
        const column = document.createElement('div');
        column.className = 'data-column';
        
        // Layer-specific styling for depth effect
        if (layer === 'background') {
            column.classList.add('data-column-background');
        }
        
        // Random horizontal position
        column.style.left = (Math.random() * window.innerWidth) + 'px';
        
        // Pick a random code snippet from the global array
        const codeSnippet = window.codeSnippets[Math.floor(Math.random() * window.codeSnippets.length)];
        column.textContent = codeSnippet;
        
        // Different speeds for depth perception
        let duration;
        if (layer === 'background') {
            // Background: MUCH slower (25-40 seconds)
            duration = (Math.random() * 15 + 25) + 's';
        } else {
            // Foreground: Normal speed (12-25 seconds)
            duration = (Math.random() * 13 + 12) + 's';
        }
        column.style.animationDuration = duration;
        
        // Random delay for staggered start
        const delay = (Math.random() * 10) + 's';
        column.style.animationDelay = delay;
        
        dataStream.appendChild(column);
        
        // Remove and recreate after animation completes
        const totalDuration = (parseFloat(duration) + parseFloat(delay)) * 1000;
        setTimeout(() => {
            column.remove();
            createColumn(index, layer);
        }, totalDuration);
    }
}

// Title appears immediately - no animation needed

// Initialize on load
document.addEventListener('DOMContentLoaded', () => {
    initMatrixEffect();
    
    // Smooth scroll for navigation links
    document.querySelectorAll('a[href^="#"]').forEach(anchor => {
        anchor.addEventListener('click', function (e) {
            e.preventDefault();
            const target = document.querySelector(this.getAttribute('href'));
            if (target) {
                target.scrollIntoView({
                    behavior: 'smooth',
                    block: 'start'
                });
            }
        });
    });
});

// Recreate columns on window resize (debounced)
let resizeTimeout;
window.addEventListener('resize', () => {
    clearTimeout(resizeTimeout);
    resizeTimeout = setTimeout(() => {
        const dataStream = document.getElementById('dataStream');
        dataStream.innerHTML = '';
        initMatrixEffect();
    }, 500);
});
