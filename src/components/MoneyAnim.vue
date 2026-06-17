<template>
    <div class="profileEmptyAnimation">
        <div class="pulseRing"></div>
        <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M12 2v20M17 5H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6" />
        </svg>
    </div>
</template>

<style scoped>
.profileEmptyAnimation {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 44px;
    height: 44px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.03);
    color: hsl(var(--accent-h), var(--accent-s), 72%);
    margin-bottom: 12px;
}
.pulseRing {
    position: absolute;
    inset: -6px;
    border: 1.5px solid hsla(var(--accent-h), var(--accent-s), 72%, 0.3);
    border-radius: 50%;
    animation: ringPulse 2.5s infinite ease-out;
}
@keyframes ringPulse {
    0% { transform: scale(0.9); opacity: 0.9; }
    100% { transform: scale(1.3); opacity: 0; }
}
</style>