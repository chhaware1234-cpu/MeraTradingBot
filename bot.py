import yfinance as yf
import requests
import os
import warnings
import datetime
warnings.filterwarnings('ignore')

TELEGRAM_TOKEN = os.environ.get('TELEGRAM_TOKEN')
CHAT_ID = os.environ.get('CHAT_ID')

# ========================================================
# 💰 AAPKE ACCOUNT KI SETTINGS 
# ========================================================
SPOT_CAPITAL = 20      # Spot trade mein $20 lagayenge
FUTURES_MARGIN = 50    # Futures mein $50 margin
LEVERAGE = 10          # 10x Leverage

def send_telegram_alert(message):
    if not TELEGRAM_TOKEN or not CHAT_ID: return
    url = f"https://api.telegram.org/bot{TELEGRAM_TOKEN}/sendMessage"
    payload = {"chat_id": CHAT_ID, "text": message}
    try:
        requests.post(url, data=payload)
    except Exception: pass

def scan_market(ticker, name, interval, period, style):
    try:
        d = yf.download(ticker, period=period, interval=interval, progress=False).dropna()
        if len(d) < 200: return 

        close = d["Close"].squeeze()
        open_p = d["Open"].squeeze()
        high = d["High"].squeeze()
        low = d["Low"].squeeze()
        volume = d["Volume"].squeeze()
        
        # --- INDICATORS ---
        ema9 = close.ewm(span=9).mean().iloc[-1]
        ema21 = close.ewm(span=21).mean().iloc[-1]
        ema200 = close.ewm(span=200).mean().iloc[-1] 
        
        delta = close.diff()
        gain = delta.clip(lower=0).rolling(14).mean().iloc[-1]
        loss = -delta.clip(upper=0).rolling(14).mean().iloc[-1]
        rsi = 100 - (100 / (1 + gain/loss)) if loss != 0 else 100
        
        macd_line = close.ewm(span=12).mean() - close.ewm(span=26).mean()
        signal_line = macd_line.ewm(span=9).mean()
        macd_bullish = macd_line.iloc[-1] > signal_line.iloc[-1]
        
        tr = high - low
        atr = tr.rolling(14).mean().iloc[-1]
        
        price = round(float(close.iloc[-1]), 4)
        rsi = round(float(rsi), 1)

        # --- VOLUME & S/R FILTERS ---
        current_vol = volume.iloc[-1]
        avg_vol = volume.rolling(20).mean().iloc[-1]
        vol_ratio = current_vol / avg_vol if avg_vol != 0 else 1
        
        recent_high = high.tail(30).max()
        recent_low = low.tail(30).min()
        
        sr_margin = 0.998 if (".NS" in ticker or ticker == "GC=F") else 0.995
        near_resistance = price >= (recent_high * sr_margin)
        near_support = price <= (recent_low * (2 - sr_margin))

        # --- CANDLE PATTERN ---
        c_open, c_close = open_p.iloc[-1], close.iloc[-1]
        p_open, p_close = open_p.iloc[-2], close.iloc[-2]
        bull_engulf = (c_close > c_open) and (p_close < p_open) and (c_close > p_open) and (c_open < p_close)
        bear_engulf = (c_close < c_open) and (p_close > p_open) and (c_close < p_open) and (c_open > p_close)
        candle_msg = "🕯️ Pattern" if (bull_engulf or bear_engulf) else ""

        # --- BUY LOGIC ---
        if ema9 > ema21 and price > ema200 and macd_bullish and rsi < 70 and vol_ratio >= 0.5:
            if not near_resistance:
                strength = "STRONG 🔥" if (bull_engulf and vol_ratio > 1.5) else "GOOD 👍"
                sl = round(price - (atr * 1.5), 4)
                tp = round(price + (atr * 3.0), 4)
                
                spot_qty = round(SPOT_CAPITAL / price, 4)
                fut_qty = round((FUTURES_MARGIN * LEVERAGE) / price, 4)
                
                msg = (f"🟢 BUY SIGNAL 🟢\n"
                       f"Asset: {name} ({style})\n"
                       f"Price: {price}\n"
                       f"Strength: {strength} {candle_msg}\n"
                       f"🛑 SL: {sl} | 🎯 TP: {tp}\n\n"
                       f"🛡️ **SPOT TRADING (Hold)**\n"
                       f"Qty: {spot_qty} (Capital: ~${SPOT_CAPITAL})\n\n"
                       f"🚀 **FUTURES TRADING (Long)**\n"
                       f"Qty: {fut_qty} (Margin: ${FUTURES_MARGIN} @ {LEVERAGE}x)")
                
                send_telegram_alert(msg) 
            
        # --- SELL LOGIC ---
        elif ema9 < ema21 and price < ema200 and not macd_bullish and vol_ratio >= 0.5:
            if not (near_support and rsi < 35) and rsi > 30:
                strength = "STRONG 🔥" if (bear_engulf and vol_ratio > 1.5) else "GOOD 👍"
                sl = round(price + (atr * 1.5), 4)
                tp = round(price - (atr * 3.0), 4)
                
                fut_qty = round((FUTURES_MARGIN * LEVERAGE) / price, 4)
                
                msg = (f"🔴 SELL SIGNAL 🔴\n"
                       f"Asset: {name} ({style})\n"
                       f"Price: {price}\n"
                       f"Strength: {strength} {candle_msg}\n"
                       f"🛑 SL: {sl} | 🎯 TP: {tp}\n\n"
                       f"🚀 **FUTURES TRADING (Short)**\n"
                       f"Qty: {fut_qty} (Margin: ${FUTURES_MARGIN} @ {LEVERAGE}x)\n\n"
                       f"*(Spot mein Short nahi hota)*")
                
                send_telegram_alert(msg) 
                
    except Exception as e:
        pass 

def main():
    send_telegram_alert("🤖 Dual-Bot (Spot + Futures) scan shuru...")
    
    utc_now = datetime.datetime.utcnow()
    ist_now = utc_now + datetime.timedelta(hours=5, minutes=30)
    
    is_weekday = ist_now.weekday() < 5
    is_indian_market_open = is_weekday and (9 <= ist_now.hour < 15) and not (ist_now.hour == 15 and ist_now.minute > 30)

    assets = [
        ("BTC-USD", "BITCOIN"), ("ETH-USD", "ETHEREUM"), ("SOL-USD", "SOLANA"), 
        ("XRP-USD", "RIPPLE (XRP)"), ("BNB-USD", "BINANCE COIN"), ("DOGE-USD", "DOGECOIN"),
        ("ADA-USD", "CARDANO"), ("MATIC-USD", "POLYGON"), ("DOT-USD", "POLKADOT"),
        ("AVAX-USD", "AVALANCHE"), ("LINK-USD", "CHAINLINK"), ("GC=F", "GOLD")
    ]
    
    if is_indian_market_open:
        assets.extend([
            ("RELIANCE.NS", "RELIANCE"), ("TATAMOTORS.NS", "TATA MOTORS"),
            ("HDFCBANK.NS", "HDFC BANK"), ("INFY.NS", "INFOSYS"),
            ("TCS.NS", "TCS"), ("ICICIBANK.NS", "ICICI BANK"),
            ("SBIN.NS", "SBI"), ("BHARTIARTL.NS", "AIRTEL"),
            ("ITC.NS", "ITC"), ("LT.NS", "L&T")
        ])

    # 1. Swing (Daily)
    for t, n in assets: scan_market(t, n, "1d", "2y", "SWING")
    # 2. Intraday (1-Hour)
    for t, n in assets: scan_market(t, n, "1h", "1y", "INTRADAY")
    # 3. Scalping (15-min)
    for t, n in assets:
        if ".NS" not in t: 
            scan_market(t, n, "15m", "60d", "SCALPING")

if __name__ == "__main__":
    main()
