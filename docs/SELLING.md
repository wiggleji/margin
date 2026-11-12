# Selling Margin Pro

## Decisions

| | |
|---|---|
| Model | One-time purchase; the key unlocks Pro forever for versions released within **1 year** (`margin-license issue` default `--years 1`) |
| Price | **$19 launch price**, then **$29** regular |
| Store | **Paddle** (Merchant of Record); **Polar** as the fallback |
| Not used | Stripe directly (no merchant accounts for Korea-based sellers, and no global tax handling); Lemon Squeezy (fees stack to about 8.5% + $0.50, and Korean bank payouts are unconfirmed) |

Still open:
- When the launch price ends (a date, or a number of licences).
- Renewal price for another year of updates (around $15 was suggested).
- Regional (lower) prices.
- Team or volume pricing.

The price lives only in the store, not in the app. "Buy Margin Pro" opens `PURCHASE_URL` (`crates/margin-app/src/pro.rs`), so changing the price never needs an app release.

## What you keep (Paddle: 5% + $0.50)

| Price | Fee | You keep |
|---|---|---|
| $19 | $1.45 | $17.55 |
| $29 | $1.95 | $27.05 |

## Why a Merchant of Record

Paddle sells to the customer on Margin's behalf. It collects and remits VAT and sales tax worldwide, issues invoices (including VAT-ID invoices for companies), and handles refunds and chargebacks. It then pays out the net amount. As a Korea-based solo seller, doing that tax work yourself in every country isn't practical.

## Setup checklist

1. **Business registration in Korea** (개인사업자), at the tax office or on Hometax.
2. **Website** for Margin with a product page, pricing, terms of service, privacy policy, and refund policy. Paddle reviews these before approval.
3. **Paddle account.** Build everything in the sandbox first, then submit your domain and business for verification.
4. **Payouts** to a **USD account at a Korean bank** (or Payoneer), to avoid converting currency twice.
5. **Product** "Margin Pro": one-time price **$19** during launch, changed to **$29** afterwards.
6. **App:** set `PURCHASE_URL` to the Paddle checkout link and ship a release (see `docs/RELEASING.md`).
7. **Licence delivery:**
   - **Start:** for each Paddle order, run `cargo run -p margin-license -- issue <private-key> --name … --email …` and email the key.
   - **Later:** automate it with a webhook (Paddle "transaction completed" → sign a key → email). The signing key then lives on that server and must be protected.

## Korean tax (confirm with a 세무사)

- **Income** is the net amount Paddle pays you, not the customer's total.
- **VAT may be 0%** (부가세 영세율) for services supplied abroad. Whether it applies when you're paid in won is disputed, so consider a written ruling request (국세청 사전 서면질의).
- **File income tax (종합소득세) every May**, and keep proof of each foreign-currency deposit (외화입금증명서).
