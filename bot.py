import yfinance as yf
import requests
import os
import warnings
import datetime
warnings.filterwarnings('ignore')

TELEGRAM_TOKEN = os.environ.get('TELEGRAM_TOKEN')
CHAT_ID = os.environ.get('CHAT_ID')
RISK_PER_TRADE = 1000 

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
        
        # Indicators
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
        
        price = round(float(close.iloc[-1]), 2)
        rsi = round(float(rsi), 1)

        current_vol = volume.iloc[-1]
        avg_vol = volume.rolling(20).mean().iloc[-1]
        vol_ratio = current_vol / avg_vol if avg_vol != 0 else 1
        
        # --- NAYA: SUPPORT & RESISTANCE (Pichle 30 candles ka High/Low) ---
        recent_high = high.tail(30).max()
        recent_low = low.tail(30).min()
        
        # Agar price apne recent High ke bahut paas hai (0.5% ke andar), toh BUY nahi karega
        near_resistance = price >= (recent_high * 0.995)
        # Agar price apne recent Low ke bahut paas hai (0.5% ke andar), toh SELL nahi karega
        near_support = price <= (recent_low * 1.005)

        c_open, c_close = open_p.iloc[-1], close.iloc[-1]
        p_open, p_close = open_p.iloc[-2], close.iloc[-2]
        bull_engulf = (c_close > c_open) and (p_close < p_open) and (c_close > p_open) and (c_open < p_close)
        bear_engulf = (c_close < c_open) and (p_close > p_open) and (c_close < p_open) and (c_open > p_close)
        candle_msg = "🕯️ Pattern" if (bull_engulf or bear_engulf) else ""

        # --- BUY LOGIC (With S/R Filter) ---
        if ema9 > ema21 and price > ema200 and macd_bullish and rsi < 70 and vol_ratio >= 0.5:
            if near_resistance:
                pass # Resistance ke paas BUY nahi karna
            else:
                strength = "STRONG 🔥" if (bull_engulf and vol_ratio > 1.5) else "GOOD 👍"
                sl = round(price - (atr * 1.5), 2)
                tp = round(price + (atr * 3.0), 2)
                sl_dist = abs(price - sl)
                qty = round(RISK_PER_TRADE / sl_dist, 2) if sl_dist > 0 else 0
                
                msg = f"🟢 BUY SIGNAL 🟢\nAsset: {name} ({style})\nPrice: {price}\nQty: {qty} (Risk: ₹{RISK_PER_TRADE})\nStrength: {strength} {candle_msg}\n🛑 SL: {sl} | 🎯 TP: {tp}"
                send_telegram_alert(msg) 
            
        # --- SELL LOGIC (With S/R Filter & Overbought Bypass) ---
        elif ema9 < ema21 and price < ema200 and not macd_bullish and vol_ratio >= 0.5:
            # Naya Expert Logic: Agar RSI bahut zyada gir gaya hai (jaise 25) par price support tod raha hai, toh sell karo. 
            # Par agar price support ke bilkul upar hai (bounce aa sakta hai), toh skip karo.
            if near_support and rsi < 35:
                pass # Support par SELL nahi karna
            elif rsi > 30 or price < recent_low: 
                strength = "STRONG 🔥" if (bear_engulf and vol_ratio > 1.5) else "GOOD 👍"
                sl = round(price + (atr * 1.5), 2)
                tp = round(price - (atr * 3.0), 2)
                sl_dist = abs(sl - price)
                qty = round(RISK_PER_TRADE / sl_dist, 2) if sl_dist > 0 else 0
                
                msg = f"🔴 SELL SIGNAL 🔴\nAsset: {name} ({style})\nPrice: {price}\nQty: {qty} (Risk: ₹{RISK_PER_TRADE})\nStrength: {strength} {candle_msg}\n🛑 SL: {sl} | 🎯 TP: {tp}"
                send_telegram_alert(msg) 
                
    except Exception as e:
        pass 

def main():
    send_telegram_alert("🤖 Expert Bot Scan shuru... (With S/R Filter)")
    
    utc_now = datetime.datetime.utcnow()
    ist_now = utc_now + datetime.timedelta(hours=5, minutes=30)
    is_indian_market_open = (9 <= ist_now.hour < 15) and not (ist_now.hour == 15 and ist_now.minute > 30)

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

    for t, n in assets: scan_market(t, n, "1d", "2y", "SWING")
    for t, n in assets: scan_market(t, n, "1h", "1y", "INTRADAY")
    for t, n in assets:
        if ".NS" not in t: 
            scan_market(t, n, "15m", "60d", "SCALPING")

if __name__ == "__main__":
    main()
