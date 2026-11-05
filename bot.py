     import yfinance as yf
import requests
import os
import warnings
import datetime
from binance.client import Client
warnings.filterwarnings('ignore')

TELEGRAM_TOKEN = os.environ.get('TELEGRAM_TOKEN')
CHAT_ID = os.environ.get('CHAT_ID')
RISK_PER_TRADE = 1000 

# --- BINANCE TESTNET CONNECTION ---
BINANCE_API_KEY = os.environ.get('BINANCE_API_KEY')
BINANCE_SECRET_KEY = os.environ.get('BINANCE_SECRET_KEY')

try:
    # testnet=True ka matlab fake money use ho raha hai
    client = Client(BINANCE_API_KEY, BINANCE_SECRET_KEY, testnet=True)
except Exception as e:
    print("Binance connection failed. Check API keys.")

def send_telegram_alert(message):
    if not TELEGRAM_TOKEN or not CHAT_ID: return
    url = f"https://api.telegram.org/bot{TELEGRAM_TOKEN}/sendMessage"
    payload = {"chat_id": CHAT_ID, "text": message}
    try:
        requests.post(url, data=payload)
    except Exception: pass

def place_binance_order(symbol, side):
    try:
        # Test ke liye 0.001 BTC/ETH kharid/bech rahe hain
        if side == "BUY":
            order = client.order_market_buy(symbol=symbol, quantity=0.001)
            return f"✅ TESTNET BUY Order Success! ID: {order['orderId']}"
        elif side == "SELL":
            order = client.order_market_sell(symbol=symbol, quantity=0.001)
            return f"✅ TESTNET SELL Order Success! ID: {order['orderId']}"
    except Exception as e:
        return f"❌ Order Failed: {e}"
    return ""

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
        
        recent_high = high.tail(30).max()
        recent_low = low.tail(30).min()
        
        near_resistance = price >= (recent_high * 0.995)
        near_support = price <= (recent_low * 1.005)

        # --- STRICT BUY LOGIC ---
        if ema9 > ema21 and price > ema200 and macd_bullish and rsi < 70 and vol_ratio >= 0.5:
            if not near_resistance:
                sl = round(price - (atr * 1.5), 2)
                tp = round(price + (atr * 3.0), 2)
                
                order_status = ""
                # Sirf Crypto ke liye Testnet order lagayega (Stocks/Gold nahi)
                if ".NS" not in ticker and ticker != "GC=F":
                    binance_symbol = ticker.replace("-USD", "USDT")
                    order_status = place_binance_order(binance_symbol, "BUY")
                
                msg = f"🟢 BUY SIGNAL 🟢\nAsset: {name} ({style})\nPrice: {price}\n{order_status}\n🛑 SL: {sl} | 🎯 TP: {tp}"
                send_telegram_alert(msg) 
            
        # --- STRICT SELL LOGIC ---
        elif ema9 < ema21 and price < ema200 and not macd_bullish and vol_ratio >= 0.5:
            if not (near_support and rsi < 35) and rsi > 30:
                sl = round(price + (atr * 1.5), 2)
                tp = round(price - (atr * 3.0), 2)
                
                order_status = ""
                if ".NS" not in ticker and ticker != "GC=F":
                    binance_symbol = ticker.replace("-USD", "USDT")
                    order_status = place_binance_order(binance_symbol, "SELL")
                
                msg = f"🔴 SELL SIGNAL 🔴\nAsset: {name} ({style})\nPrice: {price}\n{order_status}\n🛑 SL: {sl} | 🎯 TP: {tp}"
                send_telegram_alert(msg) 
                
    except Exception as e:
        pass 

def main():
    send_telegram_alert("🤖 Bot Testnet Mode ON! Auto-Order Test shuru...")
    
    # Test ke liye sirf BTC aur ETH rakha hai taaki test jaldi ho
    assets = [("BTC-USD", "BITCOIN"), ("ETH-USD", "ETHEREUM")]
    
    for t, n in assets: 
        scan_market(t, n, "1h", "1y", "INTRADAY")

if __name__ == "__main__":
    main()
