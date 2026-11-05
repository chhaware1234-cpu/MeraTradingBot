import yfinance as yf
import requests
import os
import warnings
warnings.filterwarnings('ignore')

# GitHub Secrets se Token aur Chat ID uthayega
TELEGRAM_TOKEN = os.environ.get('TELEGRAM_TOKEN')
CHAT_ID = os.environ.get('CHAT_ID')

def send_telegram_alert(message):
    if not TELEGRAM_TOKEN or not CHAT_ID:
        print("Secrets missing! Setup incomplete.")
        return
    url = f"https://api.telegram.org/bot{TELEGRAM_TOKEN}/sendMessage"
    payload = {"chat_id": CHAT_ID, "text": message}
    try:
        requests.post(url, data=payload)
    except Exception as e:
        print(f"Telegram Error: {e}")

def scan_market(ticker, name, interval, period, style):
    try:
        d = yf.download(ticker, period=period, interval=interval, progress=False).dropna()
        if len(d) < 200: return 

        close = d["Close"].squeeze()
        open_p = d["Open"].squeeze()
        high = d["High"].squeeze()
        low = d["Low"].squeeze()
        volume = d["Volume"].squeeze()
        
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
        
        c_open, c_close = open_p.iloc[-1], close.iloc[-1]
        p_open, p_close = open_p.iloc[-2], close.iloc[-2]
        bull_engulf = (c_close > c_open) and (p_close < p_open) and (c_close > p_open) and (c_open < p_close)
        bear_engulf = (c_close < c_open) and (p_close > p_open) and (c_close < p_open) and (c_open > p_close)
        candle_msg = "🕯️ Pattern" if (bull_engulf or bear_engulf) else ""

        if ema9 > ema21 and price > ema200 and macd_bullish and rsi < 70 and vol_ratio >= 0.5:
            strength = "STRONG 🔥" if (bull_engulf and vol_ratio > 1.5) else "GOOD 👍"
            sl = round(price - (atr * 1.5), 2)
            tp = round(price + (atr * 3.0), 2)
            msg = f"🟢 BUY SIGNAL 🟢\nCoin: {name} ({style})\nPrice: {price}\nStrength: {strength} {candle_msg}\n🛑 SL: {sl} | 🎯 TP: {tp}"
            send_telegram_alert(msg) 
            
        elif ema9 < ema21 and price < ema200 and not macd_bullish and rsi > 30 and vol_ratio >= 0.5:
            strength = "STRONG 🔥" if (bear_engulf and vol_ratio > 1.5) else "GOOD 👍"
            sl = round(price + (atr * 1.5), 2)
            tp = round(price - (atr * 3.0), 2)
            msg = f"🔴 SELL SIGNAL 🔴\nCoin: {name} ({style})\nPrice: {price}\nStrength: {strength} {candle_msg}\n🛑 SL: {sl} | 🎯 TP: {tp}"
            send_telegram_alert(msg) 
                
    except Exception as e:
        pass 

def main():
    print("Market Scan ho rahi hai...")
    coins = [
        ("BTC-USD", "BITCOIN"), ("ETH-USD", "ETHEREUM"), ("SOL-USD", "SOLANA"), 
        ("XRP-USD", "RIPPLE (XRP)"), ("BNB-USD", "BINANCE (BNB)"), 
        ("DOGE-USD", "DOGECOIN"), ("ADA-USD", "CARDANO"), ("GC=F", "GOLD")
    ]
    for t, n in coins: scan_market(t, n, "1d", "2y", "SWING")
    for t, n in coins: scan_market(t, n, "1h", "1y", "INTRADAY")
    for t, n in coins: scan_market(t, n, "15m", "60d", "SCALPING")
    print("Scan Complete.")

if __name__ == "__main__":
    main()
