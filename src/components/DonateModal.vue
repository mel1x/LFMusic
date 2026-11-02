<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useI18n } from '../composables/useI18n';
import CloseIcon from './Icons/CloseIcon.vue';

// "Support the author": DonationAlerts (QR + link) and crypto wallets to copy,
// same methods as in mixlauncher. Flat surface like the statistics modal.

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();

const LINK_URL = 'https://dalink.to/mel1xy';

// DonationAlerts QR modules, one row per number, bit N = column N.
const QR_ROWS = [
    0x1fd2bd08f807f, 0x105fd5bfcf641, 0x1758b389c715d, 0x174b0c6a5d95d,
    0x17428eff8915d, 0x1047c0476a541, 0x1fd555555557f, 0x00067d45dcb00,
    0x07d588fdec87d, 0x0beb0f323b796, 0x1c1ceb57e46db, 0x098615aa0b6a3,
    0x0e938814dadc0, 0x08ca0d9e00bb5, 0x1e04e37e96962, 0x11c48c0a50182,
    0x06d91fd32f652, 0x08e138129db34, 0x138646f924be5, 0x09fa9f409fd1d,
    0x1c57269d5c759, 0x09e11648f8f35, 0x1bf5eaff4d9f1, 0x091255c66e717,
    0x0d53ecd76af58, 0x07199e46f9518, 0x11f5607da71f8, 0x1822378a949a6,
    0x07f9e8f83524d, 0x026ab56bb39b7, 0x1d4cdab15d2f6, 0x1a3095c5a6824,
    0x1d29068589ff0, 0x08a1855811c34, 0x134fe196ed376, 0x0196f57a2a69f,
    0x152d088fa905c, 0x0abb17d2a6091, 0x16f5e20ce0be2, 0x10167df8b510e,
    0x0fffce7de1ec7, 0x09130f44ef700, 0x1d55c854b3e7f, 0x1912154695541,
    0x17f1e87c8155d, 0x13f11c840eb5d, 0x00effa16dd55d, 0x106a0c786fa41,
    0x1e2984a961b7f,
];
const QR_N = QR_ROWS.length;

// Horizontal runs of dark modules as one path (values exceed 32 bits, so no bitwise ops).
const qrPath = QR_ROWS.map((bits, row) => {
    let d = '';
    let col = 0;
    while (col < QR_N) {
        if (Math.floor(bits / 2 ** col) % 2 === 0) { col++; continue; }
        const start = col;
        while (col < QR_N && Math.floor(bits / 2 ** col) % 2 === 1) col++;
        d += `M${start} ${row}h${col - start}v1h${start - col}z`;
    }
    return d;
}).join('');

const TRC20 = 'TD1yryydTBLre9ATmhhMjcdBxUZSuLhcYd';
const EVM = '0x10B2945C3222F9Fb27CDcDBbEfF298961Db10e96';

interface Wallet { net: string; addr: string }
interface Coin { id: string; name: string; wallets: Wallet[] }

const coins: Coin[] = [
    { id: 'usdt', name: 'USDT', wallets: [{ net: 'TRC20', addr: TRC20 }, { net: 'ERC20', addr: EVM }, { net: 'BEP20', addr: EVM }] },
    { id: 'eth', name: 'ETH', wallets: [{ net: 'ERC20', addr: EVM }] },
    { id: 'btc', name: 'BTC', wallets: [{ net: 'Bitcoin', addr: 'bc1qrhtwhv75jrh8fz6y8qc38evkxrvg7ge0nu4z3w' }] },
    { id: 'ltc', name: 'Litecoin', wallets: [{ net: 'Litecoin', addr: 'LVtmyVY3YbMMKzjxhkVu5bgNFY5T7hhbf2' }] },
    { id: 'usdc', name: 'USDC', wallets: [{ net: 'ERC20', addr: EVM }] },
    { id: 'trx', name: 'TRON', wallets: [{ net: 'TRC20', addr: TRC20 }] },
];

const shortAddr = (a: string) => `${a.slice(0, 6)}...${a.slice(-4)}`;

function coinSub(coin: Coin) {
    if (coin.wallets.length > 1) return coin.wallets.map(w => w.net).join(', ');
    const w = coin.wallets[0];
    return `${w.net} - ${shortAddr(w.addr)}`;
}

const close = () => emit('close');

/** `${coinId}:${net}` of the address copied last, for a short confirmation. */
const copied = ref<string | null>(null);
let copiedTimer: ReturnType<typeof setTimeout> | null = null;

async function copy(coin: Coin, w: Wallet) {
    try {
        await navigator.clipboard.writeText(w.addr);
    } catch {
        return;
    }
    copied.value = `${coin.id}:${w.net}`;
    if (copiedTimer) clearTimeout(copiedTimer);
    copiedTimer = setTimeout(() => { copied.value = null; }, 1400);
}

const isCopied = (coin: Coin, w?: Wallet) =>
    w ? copied.value === `${coin.id}:${w.net}` : !!copied.value?.startsWith(`${coin.id}:`);

async function openLink() {
    try {
        await openUrl(LINK_URL);
    } catch {
        window.open(LINK_URL, '_blank', 'noopener');
    }
}

function onKeydown(e: KeyboardEvent) {
    if (props.open && e.key === 'Escape') {
        e.preventDefault();
        close();
    }
}

watch(() => props.open, open => {
    if (!open) copied.value = null;
});

onMounted(() => document.addEventListener('keydown', onKeydown));
onBeforeUnmount(() => {
    document.removeEventListener('keydown', onKeydown);
    if (copiedTimer) clearTimeout(copiedTimer);
});
</script>

<template>
    <Teleport to="body">
        <Transition name="donate">
            <div v-if="open" class="donateBackdrop" @click="close">
                <div class="donateModal" role="dialog" aria-modal="true" :aria-label="t('donate.title')" @click.stop>
                    <button type="button" class="donateClose" aria-label="Закрыть" @click="close">
                        <CloseIcon width="18" height="18" />
                    </button>

                    <div class="donateHead">
                        <div class="donateKicker">LFMusic</div>
                        <div class="donateTitle">{{ t('donate.title') }}</div>
                        <div class="donateSubtitle">{{ t('donate.desc') }}</div>
                    </div>

                    <div class="donateGrid">
                        <div class="donateCol">
                            <div class="donateLabel">DonationAlerts</div>
                            <div class="linkBlock">
                                <svg class="qr" :viewBox="`-3 -3 ${QR_N + 6} ${QR_N + 6}`" shape-rendering="crispEdges" aria-hidden="true">
                                    <rect x="-3" y="-3" :width="QR_N + 6" :height="QR_N + 6" fill="#fff" />
                                    <path :d="qrPath" fill="#111113" />
                                </svg>
                                <div class="linkInfo">
                                    <div class="linkSub">{{ t('donate.linkSub') }}</div>
                                    <button type="button" class="linkBtn" @click="openLink">
                                        {{ t('donate.open') }}
                                        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                                            <path d="M7 17 17 7M8 7h9v9" />
                                        </svg>
                                    </button>
                                </div>
                            </div>
                        </div>

                        <div class="donateCol">
                            <div class="donateLabel">{{ t('donate.crypto') }}</div>
                            <div class="coinList">
                                <div
                                    v-for="coin in coins"
                                    :key="coin.id"
                                    class="coinRow"
                                    :class="{ single: coin.wallets.length === 1 }"
                                    @click="coin.wallets.length === 1 && copy(coin, coin.wallets[0])"
                                >
                                    <span class="coinIcon" aria-hidden="true">
                                        <svg v-if="coin.id === 'usdt'" viewBox="0 0 24 24" fill="currentColor">
                                            <path d="M5 5.5h14v3h-5.4v10.5h-3.2V8.5H5z" />
                                            <path d="M12 10.2c4.4 0 8 .9 8 2s-3.6 2-8 2-8-.9-8-2 3.6-2 8-2Zm0 1.1c-3.3 0-5.9.4-5.9.9s2.6.9 5.9.9 5.9-.4 5.9-.9-2.6-.9-5.9-.9Z" />
                                        </svg>
                                        <svg v-else-if="coin.id === 'eth'" viewBox="0 0 24 24" fill="currentColor">
                                            <path d="M12 2.5 17.6 12 12 15.3 6.4 12z" />
                                            <path d="M12 16.6 17.6 13.3 12 21.5 6.4 13.3z" opacity=".7" />
                                        </svg>
                                        <svg v-else-if="coin.id === 'btc'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                                            <path d="M8 6h5.2a2.9 2.9 0 0 1 0 5.8H8m0 0h6a3.1 3.1 0 0 1 0 6.2H8zm0-5.8v12M10.5 4v2m3 0V4m-3 16v-2m3 2v-2" />
                                        </svg>
                                        <svg v-else-if="coin.id === 'ltc'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
                                            <path d="M10.5 5 9 17.5h8M6.5 13.5l7-2.6" />
                                        </svg>
                                        <svg v-else-if="coin.id === 'usdc'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-linecap="round">
                                            <circle cx="12" cy="12" r="8" stroke-width="1.6" />
                                            <path d="M14.4 9.6c-.4-.9-1.3-1.4-2.4-1.4-1.4 0-2.4.8-2.4 1.9 0 2.4 5 1.4 5 3.9 0 1.1-1.1 1.9-2.6 1.9-1.2 0-2.2-.6-2.6-1.5M12 6.8v1.4m0 7.6v1.4" stroke-width="1.7" />
                                        </svg>
                                        <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round">
                                            <path d="M3 4.5 20.5 9.5 11 21zM3 4.5l12.8 6L11 21m4.8-10.5 4.7-1" />
                                        </svg>
                                    </span>
                                    <span class="coinText">
                                        <span class="coinName">{{ coin.name }}</span>
                                        <span class="coinSub" :class="{ done: isCopied(coin) }">{{ isCopied(coin) ? t('donate.copied') : coinSub(coin) }}</span>
                                    </span>
                                    <span v-if="coin.wallets.length > 1" class="netList">
                                        <button
                                            v-for="w in coin.wallets"
                                            :key="w.net"
                                            type="button"
                                            class="netBtn"
                                            :class="{ done: isCopied(coin, w) }"
                                            @click="copy(coin, w)"
                                        >{{ w.net }}</button>
                                    </span>
                                    <span v-else class="copyBtn" :class="{ done: isCopied(coin) }" v-tooltip:left="t('donate.copy')">
                                        <svg v-if="isCopied(coin)" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                                            <path d="m5 12.5 4.5 4.5L19 7.5" />
                                        </svg>
                                        <svg v-else width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                                            <rect x="8" y="8" width="12" height="12" rx="2.5" />
                                            <path d="M16 8V6.5A2.5 2.5 0 0 0 13.5 4h-7A2.5 2.5 0 0 0 4 6.5v7A2.5 2.5 0 0 0 6.5 16H8" />
                                        </svg>
                                    </span>
                                </div>
                            </div>
                        </div>
                    </div>
                </div>
            </div>
        </Transition>
    </Teleport>
</template>

<style scoped>
.donateBackdrop {
    position: fixed;
    top: 0;
    left: 0;
    width: calc(100vw / var(--overlay-scale, 1));
    height: calc(100dvh / var(--overlay-scale, 1));
    z-index: 1000001;
    display: flex;
    align-items: center;
    justify-content: center;
    background-color: hsla(0, 0%, 0%, 0.6);
    backdrop-filter: blur(8px);
    padding: 24px;
    box-sizing: border-box;
}
/* Flat surface like the statistics modal; sections are separated by spacing only. */
.donateModal {
    position: relative;
    width: 680px;
    max-width: 100%;
    max-height: 88vh;
    overflow-y: auto;
    padding: 28px 30px 26px;
    border-radius: 28px;
    background-color: var(--light-bg);
    box-shadow: 0 32px 90px hsla(0, 0%, 0%, 0.5);
    box-sizing: border-box;
    user-select: none;
}
.donateModal::-webkit-scrollbar { width: 6px; }
.donateModal::-webkit-scrollbar-thumb { background: hsla(0, 0%, 100%, 0.14); border-radius: 3px; }
.donateClose {
    position: absolute;
    top: 20px;
    right: 20px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: 9999px;
    background: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
    transition: background-color .18s ease, color .18s ease;
}
.donateClose:hover {
    background: hsla(0, 0%, 100%, 0.12);
    color: #fff;
}

.donateHead {
    display: flex;
    flex-direction: column;
    padding-right: 44px;
}
.donateKicker {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: .12em;
    text-transform: uppercase;
    color: hsla(var(--accent-h), min(var(--accent-s), 60%), 82%, 0.8);
}
.donateTitle {
    margin-top: 8px;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 26px;
    line-height: 1.15;
    font-weight: 800;
    letter-spacing: -0.01em;
    color: hsla(0, 0%, 100%, 0.96);
}
.donateSubtitle {
    margin-top: 6px;
    font-size: 14px;
    line-height: 1.5;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.6);
}

.donateGrid {
    display: grid;
    grid-template-columns: 180px minmax(0, 1fr);
    column-gap: 40px;
    margin-top: 30px;
}
.donateCol {
    min-width: 0;
    display: flex;
    flex-direction: column;
}
.donateLabel {
    margin-bottom: 14px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: .1em;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.42);
}

.linkBlock {
    display: flex;
    flex-direction: column;
    gap: 14px;
}
.qr {
    width: 168px;
    height: 168px;
    border-radius: 12px;
    display: block;
}
.linkInfo {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 10px;
}
.linkSub {
    font-size: 12px;
    line-height: 1.45;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.5);
}
.linkBtn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 13px 0 15px;
    border-radius: 999px;
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 0.16);
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.6), 85%, 1);
    font-size: 13px;
    font-weight: 700;
    cursor: pointer;
    transition: background-color .18s ease, color .18s ease;
}
.linkBtn:hover {
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 0.28);
    color: #fff;
}

.coinList {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0 -8px;
}
.coinRow {
    min-width: 0;
    display: grid;
    grid-template-columns: 36px minmax(0, 1fr) auto;
    align-items: center;
    column-gap: 12px;
    padding: 6px 8px;
    border-radius: 12px;
    transition: background-color .18s ease;
}
.coinRow.single {
    cursor: pointer;
}
.coinRow.single:hover {
    background-color: hsla(0, 0%, 100%, 0.04);
}
.coinIcon {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    background: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.85);
}
.coinIcon svg {
    width: 19px;
    height: 19px;
}
.coinText {
    min-width: 0;
    display: flex;
    flex-direction: column;
}
.coinName,
.coinSub {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.coinName {
    font-size: 13.5px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.92);
}
.coinSub {
    margin-top: 2px;
    font-size: 12px;
    color: hsla(0, 0%, 100%, 0.5);
    font-variant-numeric: tabular-nums;
}
.coinSub.done {
    color: hsl(var(--accent-h), max(min(var(--accent-s), 70%), 35%), 78%);
}

.copyBtn {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: hsla(0, 0%, 100%, 0.42);
    transition: background-color .18s ease, color .18s ease;
}
.coinRow.single:hover .copyBtn {
    color: hsla(0, 0%, 100%, 0.85);
}
.copyBtn.done,
.coinRow.single:hover .copyBtn.done {
    color: hsl(var(--accent-h), max(min(var(--accent-s), 70%), 35%), 78%);
}

.netList {
    display: flex;
    gap: 4px;
}
.netBtn {
    height: 26px;
    padding: 0 9px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: .02em;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
    transition: background-color .18s ease, color .18s ease;
}
.netBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.12);
    color: #fff;
}
.netBtn.done {
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 0.16);
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.6), 85%, 1);
}

.donate-enter-active, .donate-leave-active { transition: opacity .2s ease; }
.donate-enter-active .donateModal, .donate-leave-active .donateModal { transition: transform .24s cubic-bezier(0.32, 0.72, 0, 1), opacity .24s ease; }
.donate-enter-from, .donate-leave-to { opacity: 0; }
.donate-enter-from .donateModal, .donate-leave-to .donateModal { transform: scale(0.96) translateY(6px); opacity: 0; }

/* Phones: full-screen like the statistics modal, one column. */
@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .donateBackdrop { padding: 0; }
    .donateModal {
        width: 100%;
        max-width: none;
        height: 100%;
        max-height: none;
        border-radius: 0;
        padding: calc(var(--safe-top, 0px) + 26px) 20px calc(var(--safe-bottom, 0px) + 28px);
    }
    .donateClose { top: calc(var(--safe-top, 0px) + 18px); }
    .donateGrid {
        grid-template-columns: minmax(0, 1fr);
        row-gap: 30px;
    }
    .linkBlock {
        flex-direction: row;
        align-items: center;
        gap: 16px;
    }
    .qr {
        flex-shrink: 0;
        width: 132px;
        height: 132px;
    }
}
</style>
